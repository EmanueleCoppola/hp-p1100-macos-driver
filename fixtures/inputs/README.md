# Fixture inputs

Tiny deterministic source documents. The CUPS raster derived from them is generated on
demand (a single uncompressed Letter page at 600 dpi is ~31 MB) - see
`docs/testing.md` for the exact commands.

Planned set, once the encoder exists:

```text
blank.txt         blank page
square.svg/pdf    single black square
hline.txt         horizontal line
vline.txt         vertical line
text-1p.txt       "TEST 123"
gray.pdf          grayscale ramp
```
