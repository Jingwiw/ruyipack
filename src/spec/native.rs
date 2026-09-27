// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Source identities from already-expanded native RPM output, never a macro evaluator.

use super::ParsedSpec;
use rpm_spec::{
    ast::{Section, SpecItem, Tag},
    parse_result::Severity,
};

pub(crate) fn source_url(expanded: &str, wanted: u32) -> Result<&str, String> {
    let parsed = ParsedSpec::parse(expanded);
    if parsed
        .parsed
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error)
    {
        return Err("cannot map native RPM output: parser errors".into());
    }
    let mut next = Some(0_u32);
    let mut found = None;
    for item in &parsed.parsed.spec.items {
        match item {
            SpecItem::Preamble(item) => {
                if let Tag::Source(number) = item.tag {
                    let number = super::source_number(&mut next, number)
                        .ok_or("implicit Source number overflow")?;
                    if number == wanted {
                        if found.is_some() {
                            return Err(format!("duplicate Source{wanted} in native output"));
                        }
                        let raw = expanded
                            .get(item.data.start_byte..item.data.end_byte)
                            .ok_or("invalid Source range in native output")?;
                        found = Some(
                            raw.split_once(':')
                                .ok_or("Source header lacks a colon")?
                                .1
                                .trim(),
                        );
                    }
                }
            }
            SpecItem::Section(section)
                if matches!(section.as_ref(), Section::SourceList { .. }) =>
            {
                return Err("%sourcelist numbering is not supported by source-hash".into());
            }
            SpecItem::Conditional(_) | SpecItem::Include(_) => {
                return Err("native RPM output still contains unresolved source context".into());
            }
            _ => {}
        }
    }
    found.ok_or_else(|| format!("Source{wanted} is absent from the native main-package preamble"))
}
