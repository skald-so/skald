//! The spec's field of the account's personal space: `membership.md`, "Linking a device" and
//! "Anchors", give `link-body` and `anchors-body` an optional field 4, an ID, for the personal
//! space their signer pinned.

const MEMBERSHIP: &str = include_str!("../../../../docs/protocol/membership.md");

/// The lines of the CDDL map `<rule> = {` of `membership.md`, up to its closing `}`.
fn rule(name: &str) -> Vec<&'static str> {
    let opening = format!("{name} = {{");
    let mut lines = MEMBERSHIP
        .lines()
        .skip_while(|line| line.trim_end() != opening);
    assert!(
        lines.next().is_some(),
        "membership.md has no rule {opening:?}"
    );
    lines.take_while(|line| line.trim_end() != "}").collect()
}

/// Checks that the map `name` has one line of field 4, optional, an `id`, whose comment names the
/// personal space.
fn assert_names_the_personal_space(name: &str) {
    let lines = rule(name);
    let personal: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| line.trim_start().starts_with("? 4 =>"))
        .collect();
    assert_eq!(personal.len(), 1, "{name}: no one field `? 4`: {lines:#?}");
    let (field, comment) = personal[0]
        .split_once(';')
        .unwrap_or_else(|| panic!("{name}: field 4 has no comment: {}", personal[0]));
    let field: String = field.split_whitespace().collect();
    assert_eq!(field, "?4=>id,", "{name}: {}", personal[0]);
    assert!(
        comment.contains("personal"),
        "{name}: field 4 is not the personal space: {}",
        personal[0]
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.trim_start().starts_with("4 =>")),
        "{name}: field 4 is required: {lines:#?}"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn link_objects_name_the_personal_space_in_an_optional_field() {
    assert_names_the_personal_space("link-body");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn anchors_objects_name_the_personal_space_in_an_optional_field() {
    assert_names_the_personal_space("anchors-body");
}
