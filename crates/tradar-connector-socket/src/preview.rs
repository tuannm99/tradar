//! How received bytes are shown. Shared by the TUI session and
//! `tradar-server`.

/// How many bytes of a non-text entry to show before truncating -- a full
/// multi-line `xxd`-style dump (as `docs/architecture.md`'s design
/// describes) would need each entry to expand into several scrollable
/// lines, complicating the scroll-offset math below for a case that's
/// mostly "confirms binary data arrived" rather than something read byte
/// by byte in this grid -- see "Sai khác khi triển khai thật" in
/// `docs/backlog/socket-connector-2026-09-29.md`.
pub const HEX_PREVIEW_BYTES: usize = 32;

pub fn hex_preview(bytes: &[u8]) -> String {
    let shown = &bytes[..bytes.len().min(HEX_PREVIEW_BYTES)];
    let mut out: String = shown
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    if bytes.len() > HEX_PREVIEW_BYTES {
        out.push_str(&format!(" … ({} bytes)", bytes.len()));
    }
    out
}
