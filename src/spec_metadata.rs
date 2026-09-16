// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Shared checks for SPEC header metadata.

/// Checks a copyright year or inclusive year range.
pub(crate) fn validate_years(value: &str) -> Result<(), &'static str> {
    let year = |value: &str| {
        value.len() == 4 && value != "0000" && value.bytes().all(|byte| byte.is_ascii_digit())
    };
    let valid = match value.split_once('-') {
        Some((start, end)) => year(start) && year(end) && start <= end,
        None => year(value),
    };
    if valid {
        Ok(())
    } else {
        Err("spec.copyright-years: expected YYYY or YYYY-YYYY")
    }
}

#[cfg(test)]
mod tests {
    use super::validate_years;

    #[test]
    fn copyright_years_have_ordered_four_digit_endpoints() {
        for value in ["2026", "2025-2026", "2026-2026"] {
            assert!(validate_years(value).is_ok(), "{value}");
        }
        for value in ["", "0000", "026", "2026-2025", "2025-", "2025-2026-2027"] {
            assert!(validate_years(value).is_err(), "{value}");
        }
    }
}
