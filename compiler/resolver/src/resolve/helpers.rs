use super::*;

pub(super) fn closest_name<'a>(needle: &Symbol, candidates: &'a [Symbol]) -> Option<&'a Symbol> {
    let needle_len = needle.as_str().chars().count();
    let max_distance = match needle_len {
        0..=3 => 1,
        4..=7 => 2,
        _ => 3,
    };
    candidates
        .iter()
        .filter(|candidate| *candidate != needle)
        .map(|candidate| {
            (
                edit_distance(needle.as_str(), candidate.as_str()),
                candidate,
            )
        })
        .filter(|(distance, _)| *distance <= max_distance)
        .min_by(|(left_distance, left), (right_distance, right)| {
            left_distance
                .cmp(right_distance)
                .then_with(|| left.cmp(right))
        })
        .map(|(_, candidate)| candidate)
}

pub(super) fn edit_distance(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right.iter().enumerate() {
            let substitution = previous[right_index] + usize::from(left_char != *right_char);
            let insertion = current[right_index] + 1;
            let deletion = previous[right_index + 1] + 1;
            current[right_index + 1] = substitution.min(insertion).min(deletion);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

pub(crate) fn variant_index(def: &def::Def, name: &Symbol) -> Option<u32> {
    def.variants
        .iter()
        .position(|v| v == name)
        .map(|i| i as u32)
}

pub(super) fn method_index(def: &def::Def, name: &Symbol) -> Option<u32> {
    def.methods.iter().position(|m| m == name).map(|i| i as u32)
}

/// Searches every enum definition for a variant named `name`. Returns
/// `Err(count)` when the match isn't unique (`0` = not found, `2+` =
/// ambiguous) — used for unqualified variant patterns like bare `Custom(x)`.
pub(super) fn find_unique_variant(
    defs: &Definitions,
    file: nether_diagnostics::FileId,
    name: &Symbol,
) -> Result<(DefId, u32), usize> {
    let mut found = None;
    let mut count = 0usize;
    for id in defs.visible_in(file) {
        let def = defs.get(id);
        if def.kind != DefKind::Enum {
            continue;
        }
        if let Some(idx) = variant_index(def, name) {
            count += 1;
            found = Some((id, idx));
        }
    }
    match count {
        1 => Ok(found.unwrap()),
        n => Err(n),
    }
}
