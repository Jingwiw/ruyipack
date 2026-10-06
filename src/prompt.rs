// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Closed choices and editable values share terminal handling, not validation rules.

use inquire::{
    CustomUserError, Text,
    autocompletion::{Autocomplete, Replacement},
    validator::Validation,
};

#[derive(Clone)]
struct Choices<'a>(&'a [&'a str]);

impl Autocomplete for Choices<'_> {
    fn get_suggestions(&mut self, input: &str) -> Result<Vec<String>, CustomUserError> {
        Ok(self
            .0
            .iter()
            .filter(|value| value.contains(input))
            .map(|value| (*value).to_owned())
            .collect())
    }

    fn get_completion(
        &mut self,
        input: &str,
        selected: Option<String>,
    ) -> Result<Replacement, CustomUserError> {
        if selected.is_some() {
            return Ok(selected);
        }
        let suggestions = self.get_suggestions(input)?;
        Ok((suggestions.len() == 1).then(|| suggestions[0].clone()))
    }
}

pub(crate) fn choose(
    message: &str,
    choices: &[&str],
    current: Option<&str>,
) -> Result<String, inquire::InquireError> {
    Text::new(message)
        .with_initial_value(current.unwrap_or(""))
        .with_autocomplete(Choices(choices))
        .with_help_message("Type to filter; Tab completes; ↑/↓ selects; Enter confirms")
        .with_validator(move |value: &str| {
            Ok(if choices.contains(&value) {
                Validation::Valid
            } else {
                Validation::Invalid(format!("Choose one of: {}", choices.join(", ")).into())
            })
        })
        .prompt()
}

pub(crate) fn value(message: &str, current: &str) -> Result<String, inquire::InquireError> {
    Text::new(&format!("{message}:"))
        .with_initial_value(current)
        .prompt()
}
