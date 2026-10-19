use super::*;
impl Emitter {
    pub(super) fn operation(&self, op: &Operation, target: &Type, f: &Function) -> String {
        let operand = |v: &Operand| match v {
            Operand::Address(p) => self.view(p, f),
            _ => self.operand(v),
        };
        match op {
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
