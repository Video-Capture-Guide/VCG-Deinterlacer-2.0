# Flag assets

The emblem flags used as progress-bar fills. Each one is a 500-pixel-wide
PNG rendered by Wikimedia Commons from the public-domain SVG listed below,
downloaded on 15 October 2026, and the last five on 17 October 2026.

Only flags that cannot be drawn from a list of colors live here. Everything
that is bands of color — France, Germany, the rainbow and the rest — is
drawn by `flags.rs` from constants and has no asset.

## Where each one came from

| File | Commons source |
| --- | --- |
| `canada.png` | [Flag of Canada (Pantone).svg](https://commons.wikimedia.org/wiki/File:Flag_of_Canada_(Pantone).svg) |
| `quebec.png` | [Flag of Quebec.svg](https://commons.wikimedia.org/wiki/File:Flag_of_Quebec.svg) |
| `south-korea.png` | [Flag of South Korea.svg](https://commons.wikimedia.org/wiki/File:Flag_of_South_Korea.svg) |
| `india.png` | [Flag of India.svg](https://commons.wikimedia.org/wiki/File:Flag_of_India.svg) |
| `taiwan.png` | [Flag of the Republic of China.svg](https://commons.wikimedia.org/wiki/File:Flag_of_the_Republic_of_China.svg) |
| `israel.png` | [Flag of Israel.svg](https://commons.wikimedia.org/wiki/File:Flag_of_Israel.svg) |
| `greece.png` | [Flag of Greece.svg](https://commons.wikimedia.org/wiki/File:Flag_of_Greece.svg) |
| `japan.png` | [Flag of Japan.svg](https://commons.wikimedia.org/wiki/File:Flag_of_Japan.svg) |
| `mexico.png` | [Flag of Mexico.svg](https://commons.wikimedia.org/wiki/File:Flag_of_Mexico.svg) |
| `iran-lion-sun.png` | [State flag of Iran (1964–1980).svg](https://commons.wikimedia.org/wiki/File:State_flag_of_Iran_(1964%E2%80%931980).svg) |
| `ontario.png` | [Flag of Ontario.svg](https://commons.wikimedia.org/wiki/File:Flag_of_Ontario.svg) |
| `usa.png` | [Flag of the United States.svg](https://commons.wikimedia.org/wiki/File:Flag_of_the_United_States.svg) |
| `united-kingdom.png` | [Flag of the United Kingdom (3-5).svg](https://commons.wikimedia.org/wiki/File:Flag_of_the_United_Kingdom_(3-5).svg) |
| `philippines.png` | [Flag of the Philippines.svg](https://commons.wikimedia.org/wiki/File:Flag_of_the_Philippines.svg) |
| `uae.png` | [Flag of the United Arab Emirates.svg](https://commons.wikimedia.org/wiki/File:Flag_of_the_United_Arab_Emirates.svg) |

National flags are in the public domain as insignia; the Commons pages above
carry each file's own licence tag.

## Replacing or adding one

Render the SVG to a PNG about 500 pixels wide, drop it in here, and add a
row to `FLAGS` in `crates/vcg-gui/src/flags.rs` with an `include_bytes!` of
the new file. Nothing else needs changing: the assets are compiled into the
binary, so there is no install step and no data-files list to keep in step
with this folder.

Add the source to the table above at the same time. `every_emblem_asset_is_
a_real_png` will catch a file that did not download — a rate-limited fetch
from Commons writes an HTML error page over the destination, which is the
right size and the wrong thing, and four of these arrived that way on the
first pass.
