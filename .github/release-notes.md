**A new version of VCG Deinterlacer, rebuilt from scratch in Rust.**

Version 1.8 was a single large Python program. This is a native Windows
application: it starts immediately, unpacks nothing to a temp folder on launch,
and installs nothing into your Python. It also does a good deal more than the
version before it.

## Download

Download **`VCG-Deinterlacer-2.0.0-Windows-x64.zip`** from the Assets list
below, unzip it anywhere, and run **VCG Deinterlacer.exe**.

> Not the two **Source code** downloads. GitHub adds those to every release
> automatically and they contain the source only — no application.

On first run the app downloads the support payload it needs — FFmpeg and a
portable VapourSynth with its plugins, about 158 MiB. That download is checked
against a byte count and a SHA-256 built into the app, and is extracted to a
staging folder and only swapped into place once it is complete, so an
interrupted download cannot leave you without a working install.

The exe is **not code-signed**, so Windows SmartScreen and Microsoft Defender
will warn about it the first time you run it. The SHA-256 of the zip is
attached as `checksum.txt` if you want to check what you downloaded.

Requires 64-bit Windows 10 or 11.

## What it does

Deinterlaces VHS, Hi8, MiniDV and other tape captures with QTGMC, the
highest-quality software deinterlacer there is. It measures your capture, tells
you what it found and what it recommends doing about it, and writes an archival
master. Every recommendation can be overridden, and every number behind one can
be read.

## New in this version

**Automatic cropping, measured off your own frames.** Other tools crop a fixed
number of columns — BT.601 takes 8 from each side whatever the file looks like.
Real capture-card blanking is rarely symmetric; on one capture measured here it
was 14 columns on *both* sides, so the preset would have left blanking inside
the picture. Three different things sit at the frame edge and each is now
detected separately: flat black blanking, the analog edge ramp that climbs
inward past it, and the torn head-switching band along the bottom of a VHS
frame, which is not dark and which brightness-based cropping cannot see at all.

**Field order is measured, not assumed.** It is the single most consequential
fact about an interlaced file — fed the wrong one, QTGMC interpolates each
field against the neighbour on the wrong side of it in time and the result
looks worse than the source. Most tape captures declare nothing, so it is now
measured from the picture and shown so you can overrule it.

**A least-destructive QTGMC profile.** QTGMC's only job here is to rebuild the
fields faithfully. Cleaning, sharpening and stabilising are separate, optional,
visible steps rather than things that happen quietly inside the deinterlacer.
Output is double-rate — 50 or 59.94 progressive frames a second, one per field
— so no motion is discarded.

**Three modes.** *Just deinterlace* does the structural work and nothing else,
for a master you intend to grade in Resolve or Topaz; it is enforced by test,
so it cannot enable an optional filter even if you explored the settings first
and then switched back. *Auto-fix* applies what the analysis recommends. *Full
control* gives you eleven settings pages with before/after previews.

**Two QTGMC implementations.** Either havsfunc or vs-jetpack's modern rewrite,
driven from the same settings, so the choice is between the libraries rather
than between two different configurations.

**Real BM3D denoising.** Block-matching in three dimensions, running after the
deinterlacer on progressive frames, picking the fastest build present on your
machine — CUDA on an NVIDIA card, otherwise an AVX2 build about four times
quicker than the reference one.

**Say what the recording is.** Before exporting you can record what is on the
tape, when it was recorded and where, and it is written *into the finished
video* rather than into a note beside it. It survives copying, emailing and
re-muxing, and shows up in Explorer, in MediaInfo and in a search across the
drive. For anyone digitising a family archive, this is the difference between a
folder of files and an archive someone else can still make sense of later.

**What was done to the file travels with it.** A one-line summary in the file's
own metadata, a JSON sidecar with the complete configuration for treating the
next tape from the same deck identically, a log of what the tools actually
said, and the generated VapourSynth script so the render can be reproduced by
hand. All four come from the same configuration the render used, so none of them
can describe a setting that was not applied.

**Everything measured is shown.** Scan type, field order, frame edges, levels,
colour cast and saturation, noise, edge halos, chroma bleeding and how many
real audio channels there are — each with the figures behind the verdict, and a
measurement the app does not trust is never acted on automatically.

**Three output formats, with figures measured rather than guessed.** ProRes 422
HQ at 103–110 Mbps for editing, FFV1 at 71–83 Mbps for keeping — mathematically
lossless, and *smaller* than the ProRes — and H.264 at CRF 18 for sharing. The
pipeline processes at 16-bit and dithers down to each codec's depth with error
diffusion, so banding from the deinterlacer's own arithmetic does not reach the
file.

**An About page and an update check.** Every setting the app uses, with the
reason for each, in one place — plus a button that asks GitHub whether there is
a newer version.

See the [README](https://github.com/Video-Capture-Guide/vcg-deinterlacer-rust#readme)
for the full detail on any of this.
