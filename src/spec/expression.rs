// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Bounded, side-effect-free evaluation over the parser's expression AST.
//! Unknown environment macros are not assumed absent. This is not an RPM runtime.

use rpm_spec::{
    ast::{
        BinOp, ConcatPart, CondExpr, CondKind, ConditionalMacro, ExprAst, MacroDef, MacroDefKind,
        MacroKind, Span, Text, TextSegment,
    },
    parser::{Input, ParserState, text::parse_text},
};
use std::{cell::Cell, collections::BTreeMap};

pub(crate) struct Context {
    // Definitions are stacked: %undefine restores the previous definition.
    definitions: BTreeMap<String, Vec<Result<Text, String>>>,
    remaining: Cell<usize>,
}

impl Default for Context {
    fn default() -> Self {
        Self {
            definitions: BTreeMap::new(),
            remaining: Cell::new(100_000),
        }
    }
}

impl Context {
    fn step(&self) -> Result<(), String> {
        // Output/depth limits alone do not bound exponentially repeated empty macros.
        let remaining = self
            .remaining
            .get()
            .checked_sub(1)
            .ok_or("static macro evaluation exceeds the 100000-step budget")?;
        self.remaining.set(remaining);
        Ok(())
    }

    pub(crate) fn from_defines(defines: &[String]) -> Result<Self, String> {
        let mut context = Self::default();
        for definition in defines {
            let (name, body) = definition
                .split_once(char::is_whitespace)
                .ok_or("--define expects NAME EXPR")?;
            if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                return Err("--define expects an identifier and a static expression".into());
            }
            context.push(name, parse(body.trim())?);
        }
        Ok(context)
    }

    fn push(&mut self, name: &str, body: Text) {
        self.definitions
            .entry(name.to_owned())
            .or_default()
            .push(Ok(body));
    }

    pub(crate) fn literal(&mut self, name: &str, value: Result<String, String>) {
        self.definitions
            .entry(name.to_owned())
            .or_default()
            .push(value.map(Text::from));
    }

    pub(crate) fn define(&mut self, definition: &MacroDef<Span>) -> Result<(), String> {
        if matches!(definition.kind, MacroDefKind::Undefine) {
            // An unobserved lower definition is unknown, not proof of absence.
            self.definitions
                .entry(definition.name.clone())
                .or_default()
                .pop();
        } else if definition.opts.is_some() || definition.literal || definition.one_shot {
            self.literal(
                &definition.name,
                Err(format!(
                    "unsupported definition of macro {}",
                    definition.name
                )),
            );
        } else if matches!(definition.kind, MacroDefKind::Global) || definition.eager {
            let expanded = self.expand(&definition.body);
            let result = expanded.as_ref().map(|_| ()).map_err(Clone::clone);
            self.literal(&definition.name, expanded);
            result?;
        } else {
            self.push(&definition.name, definition.body.clone());
        }
        Ok(())
    }

    pub(crate) fn expand_str(&self, value: &str) -> Result<String, String> {
        self.expand(&parse(value)?)
    }

    pub(crate) fn expand(&self, text: &Text) -> Result<String, String> {
        let mut output = String::new();
        self.expand_into(text, &mut Vec::new(), 0, &mut output)?;
        Ok(output)
    }

    fn expand_into<'a>(
        &'a self,
        text: &'a Text,
        stack: &mut Vec<&'a str>,
        depth: usize,
        output: &mut String,
    ) -> Result<(), String> {
        self.step()?;
        if depth >= 64 {
            return Err("excessively nested static macro expression".into());
        }
        for segment in &text.segments {
            self.step()?;
            match segment {
                TextSegment::Literal(value) => {
                    // Bound total expansion, not just each recursive fragment.
                    if output.len().saturating_add(value.len()) > 1024 * 1024 {
                        return Err("macro expansion exceeds the 1 MiB static limit".into());
                    }
                    output.push_str(value);
                }
                TextSegment::Macro(reference)
                    if matches!(reference.kind, MacroKind::Plain | MacroKind::Braced)
                    && reference.args.is_empty() => {
                        let name = reference.name.as_str();
                        let definition = self.definitions.get(name).and_then(|defs| defs.last())
                            .ok_or_else(|| format!("unsupported source macro {name:?}: unavailable or ambiguous; supply its static value with --define"))?;
                        let definition = definition.as_ref().map_err(Clone::clone)?;
                        if matches!(reference.conditional, ConditionalMacro::IfNotDefined) {
                            continue;
                        }
                        if let Some(value) = &reference.with_value {
                            self.expand_into(value, stack, depth + 1, output)?;
                        } else {
                            if stack.contains(&name) {
                                return Err(format!("cyclic or excessively nested macro {name:?}"));
                            }
                            stack.push(name);
                            let result = self.expand_into(definition, stack, depth + 1, output);
                            stack.pop();
                            result?;
                        }
                    }
                _ => return Err("unsupported Source expression: dynamic or parameterized macros are not executed".into()),
            }
        }
        Ok(())
    }

    pub(crate) fn condition(&self, kind: CondKind, expr: &CondExpr<Span>) -> Result<bool, String> {
        match (kind, expr) {
            (CondKind::If | CondKind::Elif, CondExpr::Parsed(expr)) => {
                self.number(expr, 0).map(|n| n != 0)
            }
            (CondKind::If | CondKind::Elif, CondExpr::Raw(text)) => {
                integer(&self.expand(text)?).map(|n| n != 0)
            }
            (_, CondExpr::ArchList(items)) => {
                let name = match kind {
                    CondKind::IfArch | CondKind::IfNArch | CondKind::ElifArch => "_target_cpu",
                    CondKind::IfOs | CondKind::IfNOs | CondKind::ElifOs => "_target_os",
                    _ => return Err("unsupported conditional kind".into()),
                };
                let target = self.expand_str(&format!("%{{{name}}}"))?;
                let mut found = false;
                for item in items {
                    found |= self
                        .expand(item)?
                        .split_whitespace()
                        .any(|value| value == target);
                }
                Ok(found != matches!(kind, CondKind::IfNArch | CondKind::IfNOs))
            }
            _ => Err("unsupported conditional expression".into()),
        }
    }

    fn number(&self, expr: &ExprAst<Span>, depth: usize) -> Result<i64, String> {
        self.step()?;
        if depth >= 64 {
            return Err("excessively nested static condition".into());
        }
        match expr {
            ExprAst::Integer { value, .. } => Ok(*value),
            ExprAst::Macro { text, .. } => integer(&self.expand_str(text)?),
            ExprAst::NumericConcat { parts, .. } => {
                let mut value = String::new();
                for part in parts {
                    match part {
                        ConcatPart::Literal { text, .. } => value.push_str(text),
                        ConcatPart::Macro { text, .. } => value.push_str(&self.expand_str(text)?),
                        _ => return Err("unsupported numeric concatenation".into()),
                    }
                }
                integer(&value)
            }
            ExprAst::Paren { inner, .. } => self.number(inner, depth + 1),
            ExprAst::Not { inner, .. } => Ok(i64::from(self.number(inner, depth + 1)? == 0)),
            ExprAst::Binary { kind, lhs, rhs, .. } => {
                if let (ExprAst::String { value: left, .. }, ExprAst::String { value: right, .. }) =
                    (lhs.as_ref(), rhs.as_ref())
                {
                    let left = self.expand_str(left)?;
                    let right = self.expand_str(right)?;
                    if left.contains(['"', '\\']) || right.contains(['"', '\\']) {
                        return Err("expanded condition changes quoting".into());
                    }
                    return match kind {
                        BinOp::Eq => Ok(i64::from(left == right)),
                        BinOp::Ne => Ok(i64::from(left != right)),
                        _ => Err("only equality is supported for string conditions".into()),
                    };
                }
                let left = self.number(lhs, depth + 1)?;
                let right = self.number(rhs, depth + 1)?;
                let result = match kind {
                    BinOp::LogOr => left != 0 || right != 0,
                    BinOp::LogAnd => left != 0 && right != 0,
                    BinOp::Eq => left == right,
                    BinOp::Ne => left != right,
                    BinOp::Lt => left < right,
                    BinOp::Gt => left > right,
                    BinOp::Le => left <= right,
                    BinOp::Ge => left >= right,
                };
                Ok(i64::from(result))
            }
            _ => Err("unsupported numeric condition".into()),
        }
    }
}

pub(crate) fn substitute_fields(value: &str, fields: &[(&str, &str)]) -> Result<String, String> {
    let mut context = Context::default();
    for (name, value) in fields {
        let parsed = parse(value)?;
        context.literal(
            name,
            parsed
                .literal_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("package field {name:?} is not a supported static literal")),
        );
    }
    context.expand_str(value)
}

fn integer(value: &str) -> Result<i64, String> {
    value
        .trim()
        .parse()
        .map_err(|_| format!("condition is not a supported integer: {value:?}"))
}

/// Parse a complete expression without evaluating macros or accepting parser recovery.
pub(super) fn parse(value: &str) -> Result<Text, String> {
    let state = ParserState::new();
    let (rest, text) = parse_text(&state, Input::new(value), &|_| false)
        .map_err(|_| "unsupported or invalid RPM source expression".to_owned())?;
    if !rest.is_empty() || !state.diagnostics.borrow().is_empty() {
        return Err("unsupported or invalid RPM source expression".into());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_macro_amplification_is_bounded_without_a_cycle_or_large_output() {
        let mut context = Context::default();
        context.push("m0", Text::from(String::new()));
        for index in 1..36 {
            let previous = index - 1;
            context.push(
                &format!("m{index}"),
                parse(&format!("%{{m{previous}}}%{{m{previous}}}")).unwrap(),
            );
        }
        assert!(
            context
                .expand_str("%{m35}")
                .unwrap_err()
                .contains("step budget")
        );
    }
}
