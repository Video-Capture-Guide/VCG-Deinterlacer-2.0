## New in 2.0.5

**Film transfers now come out clean, and you can overrule the film check.**

A tape made from film carries 3:2 pulldown — four film frames spread over
five video frames — and the app has always detected that and removed it,
restoring the original 23.976fps. What it did not do was finish the job. A
film transfer is rarely film all the way through: titles, credits, effects
shots and concert inserts are routinely finished at the video rate and cut
into the same tape, and a scene change can land mid-cadence. The field match
flags every frame it cannot rebuild from one film frame's two fields, and
those frames were going out still combed — on a telecined music video,
15% of them, spread evenly over the whole running time. That is what showed
up as stepping on a film transfer that was otherwise correct. Those frames
are now deinterlaced individually, before the duplicate frames are dropped.
Only the flagged frames pay the cost, so a transfer that really is film all
through runs as fast as it did.

**And if the check gets it wrong, you can now say so.** Film or video is a
measurement with a confidence attached, not a certainty: a film transfer
where every scene is busy can read as video, and a mostly-static interlaced
tape can read as film. The Check video page now shows which it decided and
lets you change it — straight away when it found a pulldown, and behind an
"Override detection" link when it did not, which is the case where someone
who knows their own tape previously had no way to correct it. Your choice
sticks, including when you switch modes, and the page tells you when it is
yours rather than the scan's.

## New in 2.0.4

**An export queue.** Line up as many tapes as you like and leave them: they
export one at a time, in order, and you can set the next one up while one is
running. Each job is a *snapshot* — the file, the settings, the mode and the
measured findings are copied when you add it — so changing anything in the
wizard afterwards belongs to the next video, never to one already queued.
The queue has its own page, reachable from the sidebar at any time: what is
running with live progress, what is waiting with how long each should take,
and what has finished with its size and a link straight to the folder. Jobs
can be reordered, copied or removed, and removing the one that is running
stops it and moves on to the next.

Time estimates are measured rather than guessed. The cost per second of
source comes from jobs your machine has actually finished, so until one has,
the page says "measuring" instead of inventing a number.

**Colour is now denoised, not just brightness.** Tape chroma noise was
neither measured nor removed, and the app explained why in terms that were
backwards — it said tape chroma is "already soft" so filtering it costs more
than it saves. Softness means there is little detail there to lose, which
makes chroma cheap and safe to filter; VHS records colour as a subcarrier
near 629 kHz and it arrives soft *and* noisy, which are independent facts.

The noise check could not see chroma at all: its indicator, temporal
outliers, does not move when only the colour is noisy — measured, it sits
unchanged while chroma noise is raised from nothing to severe. Inter-frame
colour difference is now read as well, judged against the luma so that scene
movement cancels out, and calibrated against real captures. Denoising runs
as two passes — brightness with a temporal radius, colour spatially only, at
half again the strength — which is both the correct treatment, since
matching colour blocks across frames is what smears them, and the cheaper
one.

## Fixed in 2.0.4

**A queue left running overnight could stop between videos.** Three tapes
queued at bedtime, and the third had not started by morning; opening the app
set it going again. The interface only redraws when something asks it to, and
finished jobs are collected during a redraw — so with nobody at the machine,
a render finished into a channel nobody was reading, was never marked done,
and never handed over to the next job. The log told the story exactly: 693
minutes elapsed against 37 minutes of actual encoding. Workers now wake the
interface themselves when they finish, which does not depend on the window
being visible — Windows stops redrawing a minimised one, which is precisely
the state an unattended batch is in.

**The Queue card in the sidebar was cut off**, showing "Fi" where its status
should be, and it only appeared once something was in the queue — so the one
place that says what is waiting was invisible exactly when you went looking
for it. It is now always there and reads correctly.

**Gaps in the middle of sentences, again.** The same defect as 2.0.3 and from
the same cause: an edit stripped the line-continuations out of five sentences
on the Export page and left each line's indentation inside the words. All are
single-spaced now, and the whole interface is checked for it.

**Waiting for the preview is no longer a dead end.** That render takes a
minute or two, and a line of grey text saying "Preparing preview" was being
read as a hang. There is now a moving bar and a line saying roughly how long
it takes and that the rest of the page is ready to read meanwhile.

## Fixed in 2.0.3

**Gaps in the middle of sentences.** Explanatory text on the Export, Check
video, Trim and About pages had long gaps mid-line that looked like badly
justified type. Nothing was being justified — the spaces were real, baked into
the text itself when an earlier edit stripped the line-continuations out of
those sentences and left each line's indentation inside the words. Fifteen
paragraphs across five pages were affected; all are now single-spaced.

**A failed export now says what went wrong.** Three separate things were
hiding the reason:

- The error box showed FFmpeg's complaint and not VapourSynth's. Both were
  captured, joined into one string with VapourSynth first, and then trimmed to
  the *last* 1200 characters — so the half that explained the failure was
  always the half discarded. Each is now kept and labelled separately.
- "All source loaders failed" didn't say why any of them failed. Each
  loader's own error is now reported, which distinguishes an unreadable file
  from a missing component.
- **A log was never written when an export failed**, even with the log option
  ticked, because the writer gave up whenever there was no finished file to
  describe. A failure now always writes one — `<name>.failed.log.txt` beside
  where the video would have gone — whatever that checkbox says, since a
  failure is the case where there is otherwise no record at all.

## Fixed in 2.0.2

**The components now actually download by themselves on first run.** They
never did. The app could only *check* whether FFmpeg and VapourSynth were
present and say "not ready" if they weren't — the download lived solely in a
command-line tool that wasn't even in the release zip. A new user got a
working window with no way to make it work, while the About page told them it
downloads on first run. It does now: the Welcome page fetches the payload
automatically with a progress bar, picks the tools up when it finishes without
needing a restart, and offers Try again if it fails.

## Fixed in 2.0.1

**The app would not start on some Intel graphics.** It crashed before the
window appeared, with nothing shown and nothing written down — the kind of
failure where a user can only report that "nothing happened". The cause was
Intel's Vulkan driver faulting during startup on HD Graphics 630; the
interface now draws through Direct3D 12 instead, which does not load that
driver at all.

If a graphics backend ever does fail, the app now remembers and tries the next
one on the following launch, so a machine with a broken driver fails at most
once. Setting `WGPU_BACKEND` to `dx12`, `vulkan` or `opengl` forces a
particular one. A crash or a failed start also now writes
`%LOCALAPPDATA%\vcg-deinterlacer\last-error.txt` and says so, instead of
vanishing.

---

**A new version of VCG Deinterlacer, rebuilt from scratch in Rust.**

Version 1.8 was a single large Python program. This is a native Windows
application: it starts immediately, unpacks nothing to a temp folder on launch,
and installs nothing into your Python. It also does a good deal more than the
version before it.

## Download

Download **`VCG-Deinterlacer-{{VERSION}}-Windows-x64.zip`** from the Assets list
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

## What the app does, introduced in 2.0

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

See the [README](https://github.com/Video-Capture-Guide/VCG-Deinterlacer-2.0#readme)
for the full detail on any of this.
