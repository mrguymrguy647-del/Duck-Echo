# Duck Echo brand

The app icon for Duck Echo Studio: the yellow duck quacking, with its
sound ringing out in echoes over rippling water. Made for this project in
the style of its siblings, Duck Quill and Duck Forge (an indigo tile with
stars, and the yellow duck).

| File | Use |
|---|---|
| `duck-echo.ico` | Windows icon (16 to 256 px; the close-up head for 48 px and below) |
| `duck-echo-icon-1024.png`, `-512`, `-256`, `-128`, `-64` | App icon, full scene |
| `duck-echo-icon-48.png`, `-32`, `-24`, `-16` | App icon, close-up head for small sizes |
| `duck-echo-icon.svg`, `duck-echo-icon-small.svg` | Sources of the two icons |

Colours, shared with the siblings: duck yellow `#FFCC2E`, bill orange
`#FF8C22`, echo cyan `#46D6FF` (the siblings' paint cyan), lavender
`#DCD5FF`, night indigo `#5546D0`, ink navy `#1B1446`.

To make the PNGs again after changing an SVG: open it in a browser at each
size and save the picture (any SVG renderer does), then join the PNGs into
the `.ico`: `convert duck-echo-icon-{16,24,32,48,64,128,256}.png duck-echo.ico`.
