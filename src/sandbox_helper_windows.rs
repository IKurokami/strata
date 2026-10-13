// SPDX-License-Identifier: MIT
//! Windows preview helper is intentionally unavailable until a real sandboxed
//! child can be supervised by a Job Object with strict limits. This entrypoint
//! must never parse untrusted user files directly in an ordinary Windows app
//! process or use cmd.exe/PowerShell as an isolation substitute.
pub(crate) fn run(_arguments: &[String]) -> Result<(), String> {
    Err("Windows preview helper requires native sandbox isolation".to_owned())
}

pub(crate) mod archive_cover {
    pub(crate) const MAX_INPUT_BYTES: u64 = 128 * 1024 * 1024;
}

#[cfg(test)]
mod tests {
    #[test]
    fn private_preview_helper_rejects_execution() {
        assert!(super::run(&["thumbnail-pdf".to_owned()]).is_err());
    }
}
