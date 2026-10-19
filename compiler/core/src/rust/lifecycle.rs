//! Explicit resource state and typed drop glue. Payloads never implement Rust
//! Drop: Nether's checked moves and cleanup boundaries control their lifetime.
use super::*;
impl Emitter {
    pub(super) fn resource(&self, ty: &Type) -> bool {
        self.destructors.plan(ty) != mir::drop_plan::Plan::None
    }
    pub(super) fn type_id(&self, ty: &Type) -> usize {
        self.types.iter().position(|t| t == ty).unwrap()
    }
    pub(super) fn view(&self, place: &Place, f: &Function) -> String {
        let (data, ty) = self.place(place, f);
        if !self.resource(&ty) {
            return data;
        }
        if matches!(ty, Type::Class { .. }) {
            return format!("V{} {{data:({data}).read()}}", self.type_id(&ty));
        }
        let (state, stamp) = self.state_place(place, f);
        format!(
            "V{} {{data:{data},state:{state},stamp:{stamp}}}",
            self.type_id(&ty)
        )
    }
    pub(super) fn state_place(&self, place: &Place, f: &Function) -> (String, String) {
        let mut ty = f.locals[place.local].ty.clone();
        let mut state = format!("s{}", place.local);
        let mut stamp = format!("std::ptr::addr_of_mut!(stamp{})", place.local);
        for (index, projection) in place.projections.iter().enumerate() {
            match projection {
                Projection::Dereference
                    if matches!(ty, Type::Class { .. }) && !self.captured_slot(f, place.local) =>
                {
                    // A class view stores heap identity, never the owner's slot flags.
                    state = "std::ptr::null_mut()".into();
                    stamp = "std::ptr::null_mut()".into();
                }
                Projection::Dereference => {
                    state = format!("l{}.read().state", place.local);
                    stamp = format!("l{}.read().stamp", place.local);
                }
                Projection::Field(i) => {
                    state = if matches!(ty, Type::Class { .. }) {
                        let prefix = Place {
                            local: place.local,
                            projections: place.projections[..index].to_vec(),
                        };
                        let data = self.place(&prefix, f).0;
                        format!("std::ptr::addr_of_mut!((*({data}).read()).state.f{i})")
                    } else {
                        format!("std::ptr::addr_of_mut!((*({state})).f{i})")
                    };
                    ty = match ty {
                        Type::Class { .. } => self.classes[&ty.class_key()].fields[*i].ty.clone(),
                        Type::Record { fields, .. } => fields[*i].ty.clone(),
                        Type::Tuple(fields) => fields[*i].clone(),
                        _ => unreachable!(),
                    };
                    stamp = "std::ptr::null_mut()".into();
                }
                Projection::Variant(i) => {
                    state = format!("std::ptr::addr_of_mut!((*({state})).v{i})");
                    let Type::Enum { variants, .. } = ty else {
                        unreachable!()
                    };
                    ty = Type::Tuple(variants[*i].fields.clone());
                    stamp = "std::ptr::null_mut()".into();
                }
                Projection::Index(_) | Projection::ConstantIndex(_) => {
                    let index = match projection {
                        Projection::Index(i) => format!("l{i}.read() as usize"),
                        Projection::ConstantIndex(i) => format!("{i}usize"),
                        _ => unreachable!(),
                    };
                    state = format!(
                        "std::ptr::addr_of_mut!((*({state})).elements).cast::<S{}>().add({index})",
                        match &ty {
                            Type::Array(t, _) => self.type_id(t),
                            _ => unreachable!(),
                        }
                    );
                    let Type::Array(element, _) = ty else {
                        unreachable!()
                    };
                    ty = *element;
                    stamp = "std::ptr::null_mut()".into();
                }
            }
        }
        (state, stamp)
    }
    pub(super) fn forget(&self, place: &Place, f: &Function) -> String {
        let ty = self.place(place, f).1;
        if !self.resource(&ty) {
            return String::new();
        }
        let (state, _) = self.state_place(place, f);
        format!("(*({state})).live=false;\n")
    }
    pub(super) fn lifecycle_types(&self) -> String {
        let mut out=String::from("struct Context { clock:u64, panicking:bool }\nimpl Context {fn raise(&mut self) {if self.panicking {std::process::abort();} self.panicking=true;} fn stamp(&mut self)->u64 {self.clock=self.clock.checked_add(1).unwrap_or_else(||std::process::abort());self.clock}}\n");
        for (id, ty) in self
            .types
            .iter()
            .enumerate()
            .filter(|(_, t)| self.resource(t))
        {
            if let Type::Class { .. } = ty {
                let payload = self.class_payload(&ty.class_key());
                let payload_id = self.type_id(&payload);
                let data = self.ty(&payload);
                let (state_ty, state_init) = if self.resource(&payload) {
                    (
                        format!("S{payload_id}"),
                        format!("{{let mut s=S{payload_id}::VACANT;s.live=true;s}}"),
                    )
                } else {
                    ("()".into(), "()".into())
                };
                out.push_str(&format!("struct H{id} {{data:MaybeUninit<{data}>,state:{state_ty},complete:bool}}\nunsafe fn new{id}()->*mut H{id} {{let heap=Box::into_raw(Box::new(H{id} {{data:MaybeUninit::uninit(),state:{state_init},complete:false}}));\n// class-allocation\nheap}}\n#[derive(Clone,Copy)] struct S{id} {{live:bool}}\nimpl S{id} {{const VACANT:Self=Self {{live:false}};const LIVE:Self=Self {{live:true}};}}\n#[derive(Clone,Copy)] struct V{id} {{data:*mut H{id}}}\n#[derive(Clone,Copy)] struct W{id} {{data:*mut *mut H{id},state:*mut S{id},stamp:*mut u64}}\nunsafe fn drop{id}(ctx:&mut Context,p:*mut *mut H{id},s:*mut S{id},fault:Fault)->Result<(),Fault> {{if !(*s).live {{return Ok(());}}(*s).live=false;let heap=p.read();let mut pending=None;\n"));
                if let Some(entry) = self.destructors.entry(ty) {
                    out.push_str(&format!("if (*heap).complete {{\n// class-destructor-entry {entry}\nif let Err(e)=f{entry}(ctx,V{id} {{data:heap}}) {{pending=Some(e);}}}}\n"));
                }
                if self.resource(&payload) {
                    out.push_str(&format!("if let Err(e)=drop{payload_id}(ctx,std::ptr::addr_of_mut!((*heap).data).cast(),std::ptr::addr_of_mut!((*heap).state),fault) {{if pending.is_none() {{pending=Some(e);}}}}\n"));
                }
                out.push_str("// class-deallocation\ndrop(Box::from_raw(heap));match pending {Some(e)=>Err(e),None=>Ok(())}}\n");
                continue;
            }
            let mut members = Vec::new();
            match ty {
                Type::Record { fields, .. } => {
                    for (i, f) in fields.iter().enumerate() {
                        if self.resource(&f.ty) {
                            members.push((format!("f{i}"), self.type_id(&f.ty), None));
                        }
                    }
                }
                Type::Tuple(fields) => {
                    for (i, f) in fields.iter().enumerate() {
                        if self.resource(f) {
                            members.push((format!("f{i}"), self.type_id(f), None));
                        }
                    }
                }
                Type::Array(t, n) => members.push(("elements".into(), self.type_id(t), Some(*n))),
                Type::Enum { variants, .. } => {
                    for (i, v) in variants.iter().enumerate() {
                        let t = Type::Tuple(v.fields.clone());
                        if self.resource(&t) {
                            members.push((format!("v{i}"), self.type_id(&t), None));
                        }
                    }
                }
                _ => unreachable!(),
            }
            out.push_str(&format!("#[derive(Clone,Copy)] struct S{id} {{live:bool,"));
            for (name, child, count) in &members {
                out.push_str(&format!(
                    "{name}:{},",
                    count.map_or(format!("S{child}"), |n| format!("[S{child};{n}]"))
                ));
            }
            out.push_str(&format!("}}\nimpl S{id} {{"));
            for (name, live) in [("VACANT", false), ("LIVE", true)] {
                out.push_str(&format!("const {name}:Self=Self {{live:{live},"));
                for (field, child, count) in &members {
                    out.push_str(&format!(
                        "{field}:{},",
                        count.map_or(format!("S{child}::{name}"), |n| format!(
                            "[S{child}::{name};{n}]"
                        ))
                    ));
                }
                out.push_str("};");
            }
            out.push_str(&format!("}}\n#[derive(Clone,Copy)] struct V{id} {{data:*mut {},state:*mut S{id},stamp:*mut u64}}\n",self.ty(ty)));
            out.push_str(&format!("unsafe fn drop{id}(ctx:&mut Context,p:*mut {},s:*mut S{id},fault:Fault)->Result<(),Fault> {{if !(*s).live {{return Ok(());}} (*s).live=false;let mut pending=None;\n",self.ty(ty)));
            if let Some(entry) = self.destructors.entry(ty) {
                out.push_str(&format!("// destructor-entry {entry}\nif let Err(e)=f{entry}(ctx,V{id} {{data:p,state:s,stamp:std::ptr::null_mut()}}) {{pending=Some(e);}}\n"));
            }
            for (field, child, count) in &members {
                let (data, state) = if let Some(count) = count {
                    out.push_str(&format!("for index in 0..{count}usize {{\n"));
                    (
                        format!("p.cast::<{}>().add(index)", self.ty(&self.types[*child])),
                        format!(
                            "std::ptr::addr_of_mut!((*s).elements).cast::<S{child}>().add(index)"
                        ),
                    )
                } else if matches!(ty, Type::Enum { .. }) {
                    let variant = field.trim_start_matches('v');
                    out.push_str(&format!("if (*p).tag=={variant} {{\n"));
                    (
                        format!("std::ptr::addr_of_mut!((*p).data.{field}).cast()"),
                        format!("std::ptr::addr_of_mut!((*s).{field})"),
                    )
                } else {
                    (
                        format!("std::ptr::addr_of_mut!((*p).{field})"),
                        format!("std::ptr::addr_of_mut!((*s).{field})"),
                    )
                };
                out.push_str(&format!("if let Err(e)=drop{child}(ctx,{data},{state},fault) {{if pending.is_none() {{pending=Some(e);}}}}\n"));
                if count.is_some() || matches!(ty, Type::Enum { .. }) {
                    out.push_str("}\n");
                }
            }
            out.push_str("match pending {Some(e)=>Err(e),None=>Ok(())}}\n");
        }
        out
    }
    pub(super) fn storage(&self, f: &Function) -> String {
        let mut out = String::new();
        for (id, local) in f
            .locals
            .iter()
            .enumerate()
            .filter(|(_, l)| l.view.is_none() && self.resource(&l.ty))
        {
            let t = self.type_id(&local.ty);
            out.push_str(&format!("let mut state{id}=S{t}::VACANT;let s{id}=std::ptr::addr_of_mut!(state{id});let mut stamp{id}=0u64;\n"));
            if f.parameters.contains(&id) {
                out.push_str(&format!("s{id}.write(S{t}::LIVE);stamp{id}=ctx.stamp();\n"));
            }
        }
        out
    }
    pub(super) fn end_storage(&self, locals: &[usize], f: &Function) -> String {
        let mut out = String::from("let was_panicking=pending.is_some();\n");
        for (scope, mut selected) in f.cleanup.groups(locals) {
            selected.retain(|id| f.locals[*id].view.is_none() && self.resource(&f.locals[*id].ty));
            if selected.is_empty() {
                continue;
            }
            if f.cleanup.construction.contains(&scope) {
                selected.sort_by_key(|id| f.cleanup.field_order.get(id).copied().unwrap_or(*id));
                for id in selected {
                    out.push_str(&self.drop_local(id, f));
                }
            } else {
                out.push_str(&format!("let mut order=[{}];order.sort_unstable_by_key(|v|std::cmp::Reverse(v.0));for (_,id) in order {{match id {{\n",selected.iter().map(|id|format!("(stamp{id},{id}usize)")).collect::<Vec<_>>().join(",")));
                for id in selected {
                    out.push_str(&format!("{id}=>{{{}}},", self.drop_local(id, f)));
                }
                out.push_str("_=>unreachable!(),}}\n");
            }
        }
        let unwind=f.blocks.iter().position(|b|matches!(&b.terminator,Terminator::EndStorage {locals,next} if locals.len()==f.locals.len() && matches!(f.blocks[*next].terminator,Terminator::ResumePanic))).unwrap();
        out.push_str(&format!(
            "if !was_panicking && pending.is_some() {{pc={unwind};continue;}}\n"
        ));
        out
    }
    fn drop_local(&self, id: usize, f: &Function) -> String {
        let t = self.type_id(&f.locals[id].ty);
        format!("if let Err(e)=drop{t}(ctx,l{id},s{id},fault) {{if pending.is_none() {{pending=Some(e);}}}}\n")
    }
    pub(super) fn evaluate_resource(
        &self,
        operation: &Operation,
        destination: &Place,
        f: &Function,
        unwind: usize,
    ) -> String {
        let (dest, ty) = self.place(destination, f);
        let expression = self.operation(operation, &ty, f);
        if matches!(
            operation,
            Operation::Borrow(..) | Operation::CapturePlace(..)
        ) {
            return format!("let result:Result<(),Fault>=(||{{let value={expression};({dest}).write(value);Ok(())}})();if let Err(e)=result {{ctx.raise();pending=Some(e);pc={unwind};continue;}}\n");
        }
        let forgotten = mir::dataflow::consumed(f, operation)
            .iter()
            .map(|p| format!("let _={};{}", self.place(p, f).0, self.forget(p, f)))
            .collect::<String>();
        let assignment = if mir::returns::call_view(operation).is_some()
            || matches!(operation, Operation::LoadView(_))
        {
            "target.write(value);".into()
        } else if self.resource(&ty) {
            let t = self.type_id(&ty);
            let (state, stamp) = self.state_place(destination, f);
            format!("let state={state};if let Err(e)=drop{t}(ctx,target,state,fault) {{let mut staged=S{t}::LIVE;let _=drop{t}(ctx,std::ptr::addr_of_mut!(value),std::ptr::addr_of_mut!(staged),fault);return Err(e);}}target.write(value);state.write(S{t}::LIVE);let stamp:*mut u64={stamp};if !stamp.is_null() {{stamp.write(ctx.stamp());}}")
        } else {
            "target.write(value);".into()
        };
        let raise = if matches!(
            operation,
            Operation::Call(..)
                | Operation::CallIndirect(..)
                | Operation::CallView { .. }
                | Operation::CallStorage { .. }
        ) {
            ""
        } else {
            "if entered_panicking || !ctx.panicking {ctx.raise();}"
        };
        format!("let entered_panicking=ctx.panicking;let result:Result<(),Fault>=(||{{let target={dest};{forgotten}let mut value={expression};{assignment}Ok(())}})();if let Err(e)=result {{{raise}pending=Some(e);pc={unwind};continue;}}\n")
    }
}
