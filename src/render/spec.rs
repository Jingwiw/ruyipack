// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Complete SPEC text rendering.

use super::manifest::{Manifest, Vcs};
use crate::profile::Profile;
use std::fmt::Write as _;

/// Renders a complete SPEC from validated manifest fields and distribution defaults.
pub(crate) fn render(recipe: &Manifest, profile: &Profile) -> String {
    let mut output = String::new();
    let header = &recipe.spec;

    for holder in &profile.copyright_holders {
        writeln!(
            output,
            "# SPDX-FileCopyrightText: (C) {} {holder}",
            header.copyright_years
        )
        .expect("writing to a String cannot fail");
    }
    for contributor in &header.contributors {
        writeln!(output, "# SPDX-FileContributor: {contributor}")
            .expect("writing to a String cannot fail");
    }
    // Keep the generated marker split so REUSE does not treat it as this file's own header.
    output.push_str("#\n# SPDX-License-");
    writeln!(output, "Identifier: {}\n", profile.spec_license)
        .expect("writing to a String cannot fail");

    let column = profile.preamble_value_column;
    write_tag(&mut output, "Name:", &recipe.package.name, column);
    write_tag(&mut output, "Version:", &recipe.package.version, column);
    write_tag(&mut output, "Release:", &profile.release, column);
    write_tag(
        &mut output,
        "Summary:",
        &recipe.package.body.summary,
        column,
    );
    write_tag(&mut output, "License:", &recipe.package.license, column);
    write_tag(&mut output, "URL:", &recipe.package.url, column);
    match &recipe.package.vcs {
        Vcs::Git(url) => write_tag(&mut output, "VCS:", &format!("git:{url}"), column),
        Vcs::SameAsUrl => {}
        Vcs::NoPublicRepository => {
            writeln!(output, "{}", profile.no_public_vcs_comment)
                .expect("writing to a String cannot fail");
        }
    }
    for (number, source) in &recipe.sources {
        writeln!(output, "{}", profile.remote_asset(source.sha256.as_deref()))
            .expect("writing to a String cannot fail");
        write_tag(
            &mut output,
            &format!("Source{number}:"),
            &source.url,
            column,
        );
    }
    if recipe.package.noarch {
        write_tag(&mut output, "BuildArch:", "noarch", column);
    }
    if let Some(system) = &recipe.build.system {
        write_tag(&mut output, "BuildSystem:", system, column);
    }
    output.push('\n');

    for (stage, config) in &recipe.build.stages {
        let label = format!("BuildOption({}):", stage.as_str());
        for option in &config.options {
            // openRuyi requires two spaces after each BuildOption label.
            write_tag(&mut output, &label, option, label.len() + 2);
        }
    }
    if recipe
        .build
        .stages
        .values()
        .any(|stage| !stage.options.is_empty())
    {
        output.push('\n');
    }

    render_tag_block(
        &mut output,
        "BuildRequires:",
        &recipe.build_requires.rpm,
        column,
    );
    render_tag_block(
        &mut output,
        "Requires:",
        &recipe.package.body.requires,
        column,
    );
    render_tag_block(
        &mut output,
        "Provides:",
        &recipe.package.body.provides,
        column,
    );

    output.push_str("%description\n");
    output.push_str(recipe.package.body.description.trim_end_matches('\n'));
    output.push_str("\n\n");
    for (stage, config) in &recipe.build.stages {
        for (suffix, script) in [
            (
                " -p",
                Some(config.prepend.as_str()).filter(|s| !s.is_empty()),
            ),
            ("", config.replace.as_deref()),
            (
                " -a",
                Some(config.append.as_str()).filter(|s| !s.is_empty()),
            ),
        ] {
            let Some(script) = script else {
                continue;
            };
            writeln!(output, "%{}{suffix}", stage.as_str())
                .expect("writing to a String cannot fail");
            output.push_str(script);
            if !script.is_empty() && !script.ends_with('\n') {
                output.push('\n');
            }
            output.push('\n');
        }
    }
    output.push_str("%files\n");
    if !recipe.package.body.files.license.is_empty() {
        writeln!(
            output,
            "%license {}",
            recipe.package.body.files.license.join(" ")
        )
        .expect("writing to a String cannot fail");
    }
    if !recipe.package.body.files.doc.is_empty() {
        writeln!(output, "%doc {}", recipe.package.body.files.doc.join(" "))
            .expect("writing to a String cannot fail");
    }
    for path in &recipe.package.body.files.entries {
        writeln!(output, "{path}").expect("writing to a String cannot fail");
    }

    writeln!(output, "\n%changelog\n{}", profile.changelog)
        .expect("writing to a String cannot fail");
    output
}

fn write_tag(output: &mut String, label: &str, value: &str, column: usize) {
    writeln!(output, "{label:<column$}{value}").expect("writing to a String cannot fail");
}

/// Renders one tag line per value, then a blank line when the block is
/// non-empty. BuildRequires, Requires, and Provides share this shape, and each
/// subpackage will reuse it for its own dependency edges.
fn render_tag_block(output: &mut String, label: &str, values: &[String], column: usize) {
    for value in values {
        write_tag(output, label, value, column);
    }
    if !values.is_empty() {
        output.push('\n');
    }
}
