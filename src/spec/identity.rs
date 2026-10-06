// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Declaration-time identity for queries, not final native RPM evaluation.
use rpm_spec::{
    ast::{Span, SpecItem, Tag},
    parse_result::Severity,
};
use std::collections::BTreeMap;
fn touches(items: &[SpecItem<Span>]) -> bool {
    items.iter().any(|i| match i {
        SpecItem::Preamble(p) => matches!(p.tag, Tag::Name | Tag::Version),
        SpecItem::Conditional(c) => {
            c.branches.iter().any(|b| touches(&b.body))
                || c.otherwise.as_ref().is_some_and(|body| touches(body))
        }
        SpecItem::Include(_) | SpecItem::Statement(_) => true,
        _ => false,
    })
}
pub(crate) fn declarations(
    parsed: &super::ParsedSpec<'_>,
) -> Result<BTreeMap<&'static str, String>, String> {
    let s = parsed.source();
    let parsed = &parsed.parsed;
    if parsed
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error)
    {
        return Err("parser-error".into());
    }
    let name_count = parsed
        .spec
        .items
        .iter()
        .filter(|i| matches!(i,SpecItem::Preamble(p) if matches!(p.tag,Tag::Name)))
        .count();
    let version_count = parsed
        .spec
        .items
        .iter()
        .filter(|i| matches!(i,SpecItem::Preamble(p) if matches!(p.tag,Tag::Version)))
        .count();
    if name_count != 1 || version_count != 1 {
        return Err("identity declaration not unique".into());
    }
    if parsed.spec.items.iter().any(|i|matches!(i,SpecItem::Conditional(c) if c.branches.iter().any(|b|touches(&b.body))||c.otherwise.as_ref().is_some_and(|b|touches(b)))){return Err("conditional may alter identity".into())}
    let mut ctx = super::expression::Context::default();
    let mut values = BTreeMap::new();
    for i in &parsed.spec.items {
        match i {
            SpecItem::MacroDef(d) => ctx.define(d)?,
            SpecItem::Preamble(p) => {
                let raw = s
                    .get(p.data.start_byte..p.data.end_byte)
                    .ok_or("invalid span")?
                    .split_once(':')
                    .ok_or("no colon")?
                    .1
                    .trim();
                let key = match p.tag {
                    Tag::Name => Some("name"),
                    Tag::Version => Some("version"),
                    _ => None,
                };
                if let Some(key) = key {
                    if p.lang.is_some() || !p.qualifiers.is_empty() || values.contains_key(key) {
                        return Err("ambiguous identity".into());
                    }
                    let value = ctx.expand_str(raw)?;
                    if value.is_empty() || value.contains(['\n', '\r', '\0']) {
                        return Err("invalid identity expansion".into());
                    }
                    ctx.literal(key, Ok(value.clone()));
                    values.insert(key, value);
                    if values.len() == 2 {
                        return Ok(values);
                    }
                } else {
                    ctx.check_scalar(raw)?;
                }
            }
            SpecItem::Conditional(_) => {
                return Err("conditional identity/context requires separate proof".into());
            }
            SpecItem::Include(_) | SpecItem::Statement(_) => {
                return Err("include/top-level invocation requires separate proof".into());
            }
            SpecItem::Comment(c) if c.text.literal_str().is_none() => {
                ctx.expand(&c.text)?;
            }
            _ => {}
        }
    }
    Err("missing identity".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_uses_ordered_declarations_without_executing_later_macros() {
        let parsed = super::super::ParsedSpec::parse(
            "%global srcname timm\nName: python-%{srcname}\nVersion: 1.0\n%python_provide python3-timm\n",
        );
        let values = declarations(&parsed).unwrap();
        assert_eq!(values["name"], "python-timm");
        assert_eq!(values["version"], "1.0");
        for source in [
            "Name: %{unknown}\nVersion: 1\n",
            "Name: one\nName: two\nVersion: 1\n",
            "%if 1\nName: one\n%endif\nVersion: 1\n",
            "Name: %(echo one)\nVersion: 1\n",
        ] {
            assert!(
                declarations(&super::super::ParsedSpec::parse(source)).is_err(),
                "{source}"
            );
        }
    }
}
