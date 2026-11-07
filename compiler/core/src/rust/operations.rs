use super::*;
impl Emitter {
    pub(super) fn operation(&self, op: &Operation, target: &Type, f: &Function) -> String {
        let operand = |v: &Operand| match v {
            Operand::Address(p) => self.view(p, f),
            _ => self.operand(v),
        };
        match op {
            Operation::StorageView { arguments, .. } => {
                let Operand::Address(storage) = &arguments[0] else {
                    unreachable!()
                };
                let data = self.place(storage, f).0;
                let index = operand(&arguments[1]);
                if !self.resource(target) {
                    return format!("{{let storage={data};let index={index};if index>=(*storage).f1 {{return Err(fault);}}(*storage).f0.add(index)}}");
                }
                let id = self.type_id(target);
                format!("{{let storage={data};let index={index};if index>=(*storage).f1 {{return Err(fault);}}V{id} {{data:(*storage).f0.add(index).read()}}}}")
            }
            Operation::StoreView(p) => self.view(p, f),
            Operation::LoadView(p) => format!("({}).read()", self.place(p, f).0),
            Operation::Identity(a, b) => format!(
                "({}).read()==({}).read()",
                self.place(a, f).0,
                self.place(b, f).0
            ),
            Operation::AllocateClass(_) => format!("new{}()", self.type_id(target)),
            Operation::CompleteClass(value) => {
                format!("{{let heap={};(*heap).complete=true;heap}}", operand(value))
            }
            Operation::CapturePlace(p) if matches!(target, Type::Class { .. }) => {
                let (data, _) = self.place(p, f);
                let (state, stamp) = self.state_place(p, f);
                format!(
                    "W{} {{data:{data},state:{state},stamp:{stamp}}}",
                    self.type_id(target)
                )
            }
            Operation::Borrow(p) | Operation::CapturePlace(p) => self.view(p, f),
            Operation::Copy(p) => self.copy_expression(&self.place(p, f).0, target),
            Operation::Move(p) => format!("({}).read()", self.place(p, f).0),
            Operation::Use(Operand::Constant(v)) => self.typed_constant(v, target),
            Operation::Use(v) => operand(v),
            Operation::Cast(v, ty) => format!("({} as {})", operand(v), self.ty(ty)),
            Operation::Unary("storage_detach", value) => {
                let value = operand(value);
                let result = self.aggregate(&["a.f0".into(), "a.f1".into()], target);
                format!("{{let a={value};{result}}}")
            }
            Operation::Unary("raw_expose" | "raw_expose_mut" | "raw_recover", value) => {
                format!("({} as {})", operand(value), self.ty(target))
            }
            Operation::Unary("storage_allocate", count) => {
                let Type::Pointer { pointee, .. } = target else {
                    unreachable!()
                };
                let ty = self.ty(pointee);
                let count = operand(count);
                format!("{{let count={count};let layout=std::alloc::Layout::array::<{ty}>(count).map_err(|_|fault)?;if layout.size()==0 {{std::ptr::NonNull::<{ty}>::dangling().as_ptr()}} else {{let p=std::alloc::alloc(layout).cast::<{ty}>();if p.is_null() {{return Err(fault);}}p}}}}")
            }
            Operation::Unary(
                op @ ("raw_initialize"
                | "storage_initialize"
                | "storage_replace"
                | "storage_take"
                | "raw_take"
                | "raw_drop"
                | "storage_drop"
                | "storage_drop_range"
                | "storage_adopt"
                | "storage_release"
                | "storage_check_range"
                | "storage_move_slots"
                | "storage_address"
                | "storage_address_mut"),
                args,
            ) => {
                let Type::Tuple(fields) = self.operand_ty(args, f) else {
                    unreachable!()
                };
                let Type::Pointer { pointee, .. } = &fields[0] else {
                    unreachable!()
                };
                let args = operand(args);
                let ty = self.ty(pointee);
                let body=match *op {
                    "storage_address"=>"if a.f2>a.f1 {return Err(fault);}a.f0.add(a.f2) as *const _".into(),
                    "storage_address_mut"=>"if a.f2>a.f1 {return Err(fault);}a.f0.add(a.f2)".into(),
                    "storage_move_slots"=>"if a.f2>a.f1 || a.f6>a.f1-a.f2 || a.f5>a.f4 || a.f6>a.f4-a.f5 {return Err(fault);}std::ptr::copy(a.f3.add(a.f5),a.f0.add(a.f2),a.f6)".into(),
                    "storage_check_range"=>"if a.f2>a.f1 || a.f3>a.f1-a.f2 {return Err(fault);}".into(),
                    "raw_initialize" | "storage_initialize"=>"a.f0.add(a.f1).write(a.f2)".into(),
                    "storage_replace"=>"a.f0.add(a.f1).replace(a.f2)".into(),
                    "raw_take" | "storage_take"=>"a.f0.add(a.f1).read()".into(),
                    "storage_drop_range"=> {
                        let bounds="if a.f2>a.f1 || a.f3>a.f1-a.f2 {return Err(fault);}";
                        if self.resource(pointee) {
                            let id=self.type_id(pointee);
                            format!("{bounds}let mut pending=None;for index in a.f2..a.f2+a.f3 {{let mut state=S{id}::LIVE;if let Err(e)=drop{id}(ctx,a.f0.add(index),&mut state,fault) {{if pending.is_none() {{pending=Some(e);}}}}}}if let Some(e)=pending {{return Err(e);}}")
                        } else {bounds.into()}
                    },
                    "raw_drop" | "storage_drop"=>if self.resource(pointee) {
                        let id=self.type_id(pointee);
                        format!("let mut state=S{id}::LIVE;drop{id}(ctx,a.f0.add(a.f1),&mut state,fault)?")
                    } else {"()".into()},
                    "storage_adopt"=>format!("let _=std::alloc::Layout::array::<{ty}>(a.f1).map_err(|_|fault)?;if a.f0.is_null() || (a.f0 as usize)%std::mem::align_of::<{ty}>()!=0 {{return Err(fault);}}"),
                    "storage_release"=>format!("let layout=std::alloc::Layout::array::<{ty}>(a.f1).map_err(|_|fault)?;if layout.size()!=0 {{std::alloc::dealloc(a.f0.cast(),layout);}}"),
                    _=>unreachable!(),
                };
                format!("{{let a={args};{body}}}")
            }
            Operation::Unary("raw_addr" | "raw_addr_mut", Operand::Address(p)) => {
                format!("({} as {})", self.place(p, f).0, self.ty(target))
            }
            Operation::Unary("raw_read", p) => format!("({}).read()", operand(p)),
            Operation::Binary("raw_write", p, v) => {
                format!("({}).write({})", operand(p), operand(v))
            }
            Operation::Binary("checked_index", index, length) => {
                format!(
                    "{{let index={};if index>={} {{return Err(fault);}}index}}",
                    operand(index),
                    operand(length)
                )
            }
            Operation::Binary("raw_difference", a, b) => {
                format!("({}).offset_from({})", operand(a), operand(b))
            }
            Operation::Binary(op @ ("raw_offset_sub" | "raw_offset"), p, n) => {
                let Type::Pointer { pointee, .. } = target else {
                    unreachable!()
                };
                let pointer = operand(p);
                let amount = operand(n);
                let offset = if *op == "raw_offset_sub" {
                    "n.checked_neg().ok_or(fault)?"
                } else {
                    "n"
                };
                let zero = if crate::layout::layout(pointee).is_ok_and(|l| l.size() == 0) {
                    "if n!=0 {return Err(fault);}"
                } else {
                    ""
                };
                format!("{{let p={pointer};let n={amount};{zero}p.offset({offset})}}")
            }
            Operation::Unary(op, v) => {
                let value = operand(v);
                if *op == "-" && matches!(target, Type::Integer(_)) {
                    if self.checks == OverflowChecks::Checked {
                        format!("({value}).checked_neg().ok_or(fault)?")
                    } else {
                        format!("({value}).wrapping_neg()")
                    }
                } else if *op == "~" {
                    format!("!({value})")
                } else {
                    format!("{op}({value})")
                }
            }
            Operation::Binary(op, a, b) => {
                let left = operand(a);
                let right = operand(b);
                let ty = self.operand_ty(a, f);
                if let Type::Integer(integer) = ty {
                    let method = match *op {
                        "+" => "add",
                        "-" => "sub",
                        "*" => "mul",
                        _ => "",
                    };
                    if !method.is_empty() {
                        return if self.checks == OverflowChecks::Checked {
                            format!("({left}).checked_{method}({right}).ok_or(fault)?")
                        } else {
                            format!("({left}).wrapping_{method}({right})")
                        };
                    }
                    if *op == "/" || *op == "%" {
                        let method = if *op == "/" { "div" } else { "rem" };
                        // Division overflow traps in both Nether modes, including MIN % -1.
                        let call = format!("a.checked_{method}(b).ok_or(fault)?");
                        return format!("{{ let a = {left}; let b = {right}; if b == 0 {{ return Err(fault); }} {call} }}");
                    }
                    if *op == "<<" || *op == ">>" {
                        let signed =
                            matches!(self.operand_ty(b,f), Type::Integer(t) if t.is_signed());
                        let negative = if signed { "count < 0 || " } else { "" };
                        let method = if *op == "<<" {
                            "wrapping_shl"
                        } else {
                            "wrapping_shr"
                        };
                        let guard = if self.checks == OverflowChecks::Checked {
                            format!(
                                "if {negative}(count as u128) >= {} {{ return Err(fault); }}",
                                integer.bits()
                            )
                        } else {
                            String::new()
                        };
                        return format!("{{ let value = {left}; let count = {right}; {guard} value.{method}(count as u32) }}");
                    }
                }
                format!("({left}) {op} ({right})")
            }
            Operation::Aggregate(values) => {
                self.aggregate(&values.iter().map(operand).collect::<Vec<_>>(), target)
            }
            Operation::Enum(variant, values) => self.enum_value(
                *variant,
                &values.iter().map(operand).collect::<Vec<_>>(),
                target,
            ),
            Operation::Tag(v) => format!("({}).tag", operand(v)),
            Operation::TagPlace(p) => {
                format!("std::ptr::addr_of!((*({})).tag).read()", self.place(p, f).0)
            }
            Operation::Call(id, args)
            | Operation::CallStorage {
                function: id,
                arguments: args,
                ..
            }
            | Operation::CallView {
                function: id,
                arguments: args,
                ..
            } => format!(
                "f{id}({}{})?",
                if self.resources { "ctx," } else { "" },
                args.iter().map(operand).collect::<Vec<_>>().join(",")
            ),
            Operation::CallIndirect(callee, args, _) => format!(
                "({})({}{})?",
                operand(callee),
                if self.resources { "ctx," } else { "" },
                args.iter().map(operand).collect::<Vec<_>>().join(",")
            ),
        }
    }
}
