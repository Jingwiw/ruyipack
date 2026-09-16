// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Static Source references from explicitly supplied package fields.

use rpm_spec::{
    ast::{ConditionalMacro, MacroKind, Text, TextSegment},
    parser::{Input, ParserState, text::parse_text},
};

pub(crate) fn substitute_fields(value: &str, fields: &[(&str, &str)]) -> Result<String, String> {
    let text = parse(value)?;
    let mut resolved = String::with_capacity(value.len());
    for segment in &text.segments {
        match segment {
            TextSegment::Literal(literal) => resolved.push_str(literal),
            TextSegment::Macro(reference)
                if matches!(reference.kind, MacroKind::Plain | MacroKind::Braced)
                    && reference.conditional == ConditionalMacro::None
                    && reference.args.is_empty()
                    && reference.with_value.is_none() =>
            {
                let field = fields.iter().find_map(|(name, value)| (*name == reference.name).then_some(*value))
                    .ok_or_else(|| format!("unsupported source macro {:?}: package field is unavailable or ambiguous", reference.name))?;
                let field = parse(field)?;
                let literal = field.literal_str().ok_or_else(|| {
                    format!(
                        "unsupported source macro {:?}: package field is not a supported static literal",
                        reference.name
                    )
                })?;
                resolved.push_str(literal);
            }
            _ => return Err("unsupported Source expression requires RPM evaluation".into()),
        }
    }
    Ok(resolved)
}

fn parse(value: &str) -> Result<Text, String> {
    let state = ParserState::new();
    let (_, text) = parse_text(&state, Input::new(value), &|_| false)
        .map_err(|_| "unsupported or invalid RPM source expression".to_owned())?;
    if !state.diagnostics.borrow().is_empty() {
        return Err("unsupported or invalid RPM source expression".into());
    }
    Ok(text)
}
