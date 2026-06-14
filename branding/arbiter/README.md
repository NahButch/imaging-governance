# Arbiter — brand assets

The visual identity for **imaging-governance**: a judge's gavel — *the
deterministic engine that disposes of probabilistic model output.*

## Assets

| File | Use |
|------|-----|
| `mark.svg` / `mark.png` | primary mark, transparent (icon only) |
| `badge.svg` / `badge.png` | filled rounded-square — avatars, app icon |
| `lockup.svg` / `lockup.png` | horizontal lockup on light |
| `lockup-dark.svg` / `lockup-dark.png` | horizontal lockup on dark |
| `favicon-{16,32,48,180}.png` | favicons / Apple touch icon |
| `brand-sheet.svg` / `brand-sheet.png` | full system overview |

SVGs are the source of truth; PNGs are rendered via
`cairosvg` (see commands at the bottom).

## Palette

| Role | Hex |
|------|-----|
| Ink | `#1b2230` |
| Rust | `#d6602e` |
| Teal | `#11868a` |
| Pass | `#21a366` |
| Warn | `#e0a200` |
| Fail | `#db4d36` |

Mark = Ink head + handle base, Rust handle. On dark, the head/base go white;
the handle stays Rust. Keep clear space ≈ the height of the gavel head on all
sides. Don't recolour the gavel head to Rust or rotate the mark.

## Re-render PNGs

```bash
pip install cairosvg
python - <<'PY'
import cairosvg
for s,w,h in [("mark",480,480),("badge",512,512),("lockup",1400,360),
              ("lockup-dark",1400,360),("brand-sheet",1100,880)]:
    cairosvg.svg2png(url=f"{s}.svg", write_to=f"{s}.png", output_width=w, output_height=h)
for sz in (16,32,48,180):
    cairosvg.svg2png(url="badge.svg", write_to=f"favicon-{sz}.png", output_width=sz, output_height=sz)
PY
```
