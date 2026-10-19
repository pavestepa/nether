//! Explicit Copy allocates a fresh class identity; moves only transfer its pointer.
use super::*;
impl Emitter {
    pub(super) fn copy_expression(&self, address: &str, ty: &Type) -> String {
        if self.resource(ty) {
            format!("copy{}({address})", self.type_id(ty))
        } else {
            format!("({address}).read()")
        }
    }
    pub(super) fn class_copies(&self) -> String {
        let mut out = String::new();
        for (id, ty) in self
            .types
            .iter()
            .enumerate()
            .filter(|(_, t)| t.copyable() && self.resource(t))
        {
            let value = match ty {
                Type::Class { .. } => {
                    let payload = self.class_payload(&ty.class_key());
                    let fields = &self.classes[&ty.class_key()].fields;
                    let values = fields
                        .iter()
                        .enumerate()
                        .map(|(i, f)| {
                            self.copy_expression(
                                &format!("std::ptr::addr_of_mut!((*source).f{i})"),
                                &f.ty,
                            )
                        })
                        .collect::<Vec<_>>();
                    let state = if self.resource(&payload) {
                        format!("(*heap).state=S{}::LIVE;", self.type_id(&payload))
                    } else {
                        String::new()
                    };
                    format!("{{let source=std::ptr::addr_of_mut!((*p.read()).data).cast::<{}>();let heap=new{id}();(*heap).data.write({});{state}(*heap).complete=true;heap}}",self.ty(&payload),self.aggregate(&values,&payload))
                }
                Type::Record { fields, .. } => self.aggregate(
                    &fields
                        .iter()
                        .enumerate()
                        .map(|(i, f)| {
                            self.copy_expression(
                                &format!("std::ptr::addr_of_mut!((*p).f{i})"),
                                &f.ty,
                            )
                        })
                        .collect::<Vec<_>>(),
                    ty,
                ),
                Type::Tuple(fields) => self.aggregate(
                    &fields
                        .iter()
                        .enumerate()
                        .map(|(i, t)| {
                            self.copy_expression(&format!("std::ptr::addr_of_mut!((*p).f{i})"), t)
                        })
                        .collect::<Vec<_>>(),
                    ty,
                ),
                Type::Array(element, _) => format!(
                    "std::array::from_fn(|i| {})",
                    self.copy_expression(
                        &format!("p.cast::<{}>().add(i)", self.ty(element)),
                        element
                    )
                ),
                Type::Enum { variants, .. } => {
                    let mut arms = String::new();
                    for (index, variant) in variants.iter().enumerate() {
                        let payload = Type::Tuple(variant.fields.clone());
                        let values = variant
                            .fields
                            .iter()
                            .enumerate()
                            .map(|(i, t)| {
                                self.copy_expression(
                                    &format!("std::ptr::addr_of_mut!((*source).f{i})"),
                                    t,
                                )
                            })
                            .collect::<Vec<_>>();
                        arms.push_str(&format!("{index}=>{{let source=std::ptr::addr_of_mut!((*p).data.v{index}).cast::<{}>();{}}},",self.ty(&payload),self.enum_value(index,&values,ty)));
                    }
                    format!("match (*p).tag {{{arms}_=>unreachable!()}}")
                }
                _ => unreachable!(),
            };
            out.push_str(&format!(
                "unsafe fn copy{id}(p:*mut {})->{} {{{value}}}\n",
                self.ty(ty),
                self.ty(ty)
            ));
        }
        out
    }
}
