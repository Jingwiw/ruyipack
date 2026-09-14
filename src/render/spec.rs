// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Complete SPEC text rendering.

use super::{manifest::Manifest, profile::Profile};
use std::fmt::Write as _;

/// Renders a complete SPEC from authoring fields and distribution defaults.
pub(super) fn render(recipe: &Manifest, profile: &Profile) -> String {
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
    write_tag(&mut output, "Summary:", &recipe.package.summary, column);
    write_tag(&mut output, "License:", &recipe.package.license, column);
    write_tag(&mut output, "URL:", &recipe.package.url, column);
    writeln!(output, "{}", profile.no_public_vcs_comment).expect("writing to a String cannot fail");
    writeln!(
        output,
        "{}{}",
        profile.remote_asset_prefix, recipe.sources["0"].sha256
    )
    .expect("writing to a String cannot fail");
    write_tag(&mut output, "Source0:", &recipe.sources["0"].url, column);
    write_tag(&mut output, "BuildSystem:", &recipe.build.system, column);
    output.push('\n');

    for requirement in &recipe.build_requires.rpm {
        write_tag(&mut output, "BuildRequires:", requirement, column);
    }
    output.push('\n');

    output.push_str("%description\n");
    output.push_str(recipe.package.description.trim_end_matches('\n'));
    output.push_str("\n\n%files\n");
    if !recipe.package.files.license.is_empty() {
        writeln!(
            output,
            "%license {}",
            recipe.package.files.license.join(" ")
        )
        .expect("writing to a String cannot fail");
    }
    if !recipe.package.files.doc.is_empty() {
        writeln!(output, "%doc {}", recipe.package.files.doc.join(" "))
            .expect("writing to a String cannot fail");
    }
    for path in &recipe.package.files.entries {
        writeln!(output, "{path}").expect("writing to a String cannot fail");
    }

    writeln!(output, "\n%changelog\n{}", profile.changelog)
        .expect("writing to a String cannot fail");
    output
}

fn write_tag(output: &mut String, label: &str, value: &str, column: usize) {
    writeln!(output, "{label:<column$}{value}").expect("writing to a String cannot fail");
}
