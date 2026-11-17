# Provider brand assets

`microsoft.svg` is the official Microsoft symbol downloaded on 2026-11-17
from the Microsoft identity platform's branding guidelines:

- Guidelines: https://learn.microsoft.com/en-us/entra/identity-platform/howto-add-branding-in-apps
- Original SVG: https://learn.microsoft.com/en-us/entra/identity-platform/media/howto-add-branding-in-apps/ms-symbollockup_mssymbol_19.svg
- Local checksum: see `../provenance.json`.

The source is retained unchanged except for a final newline, including its
21 x 21 viewBox, transparent outer inset, and four original fill colors.
This is a Microsoft brand asset, not a Lucide icon; Microsoft retains its
trademark rights. Follow the linked branding guidelines, including pairing
the symbol with "Sign in with Microsoft" and not altering the logo.

Render through GPUI's full-color image route, not `svg()` or a themed icon
wrapper. The SVG element uses an alpha mask and discards the four colors
(`vendor/gpui-pre/src/svg_renderer.rs`, `render_alpha_mask`).
The encoded SVG image route preserves them through
`ImageFormat::Svg` and `render_single_frame`
(`vendor/gpui-pre/src/platform.rs`, `Image::to_image_data`).
Keep the intrinsic square aspect ratio and original fills in both themes.
