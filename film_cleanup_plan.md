# Film cleanup filters: plan

16 November 2026. Answers `claude_code_prompt_film_cleanup_filters.md`. Nothing in the product has been changed; this is the plan awaiting approval.

## Summary

- **Every filter on the list can be built from what payload v11 already ships.** No new DLLs, no installer change, no payload bump.
- **The whole chain costs about 60% more export time.** On 10 minutes of SD film that is roughly 16 minutes against 10 today (stabilisation not included; it adds about 4 more).
- **Dust removal is safe only in a guarded form.** Plain SpotLess rewrote texture across the frame and, at radius 2, erased a real mark on a sweater. The guarded form touched only specks.
- **The worst dirt in both test tapes is not on the film.** It sits in the projector gate, so it stays in one place on screen and temporal dust removal cannot touch it. That makes the static-defect item more important than "stretch goal".
- **Three pieces of plumbing have to come first**: a preview that shows restored film frames, moving the output-clock step to the end of the chain, and per-scene measurement in the existing pre-pass.

## How this was tested

A scratch copy of the app's own 2.0.8 export script, with each candidate filter added after the film frames are picked. Two sections were used:

- "8mm oldest to VHS to GVUSB2.avi", source frames 1500–2100 (20 s, dusty brick-wall scenes, pans).
- Tony tape 2a, source frames 600–1200 (indoor scene with a lit match, used as the "small bright moving object" case).

Speeds are from the first section, 640x480 with no upscale, on an idle machine. They are 20-second renders, so treat them as ratios, not promises.

- Tony tape 1a, source frames 20080–20680 (20 s from 11:10, the dusty orange indoor reel). This is the agreed test video from here on.

I did not find the exact shot in `film_artifacts_reference.png`, but the 1a section is the same kind of material.

## Decisions (16 November 2026)

- **Test video:** `E:\Video transfer of clients\Tony\Vecchi_Ricordi_8mm_Super8_Films_TAPE_1a.avi`.
- **Easy mode:** film cleanup stays off. It is chosen in Full control only.
- **Stabilisation:** left out for now, to avoid the title wobble and border problems. It can be revisited after the other filters ship.

## What the bundle already has

| Need | Provided by | Status |
|---|---|---|
| Motion compensation for dust removal | `mv.Super/Analyse/Compensate` | Present, runs at 16-bit |
| Temporal median | `zsmooth.TemporalMedian` | Present |
| Deflicker | `akarin.Expr` reading neighbours' `PlaneStatsAverage` | Present; script only, no plugin |
| Stabilisation | `haf.Stab` on `mv.DepanEstimate/DepanCompensate` | Present, ran first time |
| Film denoise | `haf.SMDegrain`; BM3D already wired | Present |
| Sharpen | `cas.CAS` | Present, not yet used by the app |
| Grain | `grain.Add`, `haf.GrainFactory3` | Present |
| Inpainting for a painted mask | nothing | Missing |

SpotLess itself is not in havsfunc or vs-jetpack. It is about 20 lines and would be emitted into the script the same way the FineDehalo port is.

## Measured cost

| Filter | Speed (fps) | Extra time on 10 min of SD film |
|---|---|---|
| Today's pipeline (bob + film frames) | 18.1 | 10.2 min total |
| Guarded dust removal | 16.5 | +1.0 min |
| Deflicker | 16.3 | +1.1 min |
| Colour restore | 16.7 | +0.9 min |
| CAS sharpen | 16.1 | +1.3 min |
| SMDegrain (tr=2) | 13.6 | +3.4 min |
| Stab | 12.8 | +4.2 min |
| Dust + deflicker + SMDegrain + colour + CAS together | 11.3 | 16.4 min total |

QTGMC is still the dominant cost. The brief's "10 minutes of SD footage" test should be rerun for real in phase 1; these figures are extrapolated from 370 frames.

## Findings per filter

### 1. Dust and dirt

- **Plain SpotLess is not shippable.** On a panning frame it altered the brick texture over most of the picture, not only the specks.
- **Radius 2 eats real detail.** On Tony 2a it removed a small dark mark on a sweater that is present in four consecutive film frames, so it is part of the scene.
- **The guarded form worked.** It keeps the temporal median only where a pixel changed by more than a threshold, grown by one pixel. It removed specks, left the brick texture alone, and kept both the sweater mark and the match flame.
- **Recommendation:** ship only the guarded form, radius 1. Strength is the threshold: Light 16, Normal 12, Strong 8 (8-bit steps). These three values are starting points and need tuning on dust-heavy footage.
- **On tape 1a (11:10–11:30)** the guarded form removed the small per-frame specks on the wall and left faces, hair and clothing untouched.
- **What it left behind on 1a is static dirt.** The speck at the left edge sits at the same screen position as on the "8mm oldest" sample, so it belongs to the transfer equipment, not to any one film.
- **Not yet tested:** a fast small object crossing a plain background (a thrown ball).

### 2. Colour and fade

- **The fade is measurable.** On the old sample the channels span R 0.17–1.11, G 0.15–0.94, B 0.12–0.69. Blue has lost about 40% of its range, which is the orange cast.
- **Full per-channel normalisation removes the cast but overshoots.** Bricks and skin look right; the dark suit turns blue-purple and whites go cool.
- **Recommendation:** an "Amount" control that blends toward the normalised picture, default about 65%, with a soft shoulder so highlights are compressed instead of clipped.
- **It must be per scene.** Tony 2a moves between orange, blue and yellow reels. One correction per file, which is what the Color tint page does today, is the wrong tool for film.
- **Presets:** "Warm cast" and "Magenta cast" can be offered as starting points, but the measured version should be the default.

### 3. Deflicker

- A script-only version runs: each frame's luma is scaled toward the average of its three neighbours on each side.
- It needs a scene-change guard, or it will smear brightness across cuts.
- I have not judged it visually. Stills cannot show flicker, and neither test section flickers badly.

### 4. Stabilisation

- `haf.Stab` runs as bundled.
- **Risk on Tony's tapes:** the transfer house burned video titles over the film ("Giacomo 6 anni"). Stabilising the picture will make those titles wobble.
- Borders need either mirroring (tested with `mirror=15`) or a small crop.
- **Decision: not in this plan's phases.** Not judged visually, and deferred until the rest has shipped.

### 5. Film denoise

- SMDegrain runs and costs about a third more time. It is motion compensated, which suits grain at 18 fps better than BM3D's block matching across large frame-to-frame motion.
- I have not compared its picture quality against the existing BM3D levels. That comparison belongs in phase 3.

### 6. Sharpen, crop, upscale

- CAS works. Crop and NNEDI3 upscale already exist and need no change.
- CAS should run before the upscale, at SD size.

### 7. Grain

- The existing grain step runs after the output clock is set. In the 29.97 output mode every video frame gets new grain, even when two video frames hold the same film frame. That is the opposite of what the brief asks.
- Fix: move the output-clock step to the end (see plumbing).

### 8. Static defects

- Both tapes have dirt that stays in one screen position: a hair hanging from the top edge and a speck at the left on the old sample, and the mid-frame hair in the reference still.
- Dust removal left all of these in place, as expected.
- A paint-a-mask page plus a fill method is the largest item here, and the bundle has no inpainting plugin.
- **Cheaper first step:** detect static defects automatically in the pre-pass (pixels that are dark outliers in the same place across many scenes), report them, and suggest a crop when they touch an edge. That handles the top-edge hair with no new UI.
- A real fill for mid-frame defects needs a research spike before I can size it. Until then, document it as a "fix in Resolve" note.

## Proposed order

The brief's order, with four changes marked.

1. QTGMC bob
2. Pick the film frames (sRestore decisions, as today)
3. **Crop the edges** (moved ahead so edge junk does not feed motion analysis or colour statistics)
4. **Deflicker** (moved ahead of dust removal; steadier brightness should give better motion vectors, to be confirmed in phase 2)
5. Guarded dust removal
6. Stabilise (optional)
7. Temporal denoise
8. Colour and fade restore, then the existing levels step
9. Square-pixel resize, **then CAS** (before the upscale)
10. NNEDI3 upscale, matrix conversion
11. Grain
12. **Output clock** (hold frames for 29.97, or conform to 18), then dither

## Plumbing that has to come first

- **Output clock last.** Today `home_movie()` picks the film frames and sets the clock in one step. Split it, so every temporal filter and the grain see each film frame exactly once.
- **Preview of restored frames.** The stage previews deliberately skip sRestore and show one bobbed field. Dust removal, deflicker and stabilisation need the neighbouring film frames. Proposal: the preview restores a short window around the chosen moment (the pre-pass on 20 s took a few seconds in the test rig) and shows one frame before and after.
- **Per-scene measurement in the pre-pass.** The pre-pass already reads every field in order. Add scene cuts, per-channel percentiles, mean brightness and a dust count to it, and apply by lookup, the same pattern sRestore now uses.

## Wizard

- **Easy mode:** no film cleanup. Everything below is off until chosen in Full control.
- **Full control:** three new steps, shown only for home movies, each defaulting to Off:
  - Film dirt (Off, Light, Normal, Strong)
  - Film colour (Off, Measured with an Amount slider, the two cast presets, saturation)
  - Flicker (Off, On)
- **Reused steps:** Noise gains a "Film grain and tape noise" option; Upscale gains a sharpen toggle; Film grain stays as it is.
- **Replaced for home movies:** Color tint (whole-file U/V shift) is hidden when Film colour is on.
- **Each page shows what was measured**, like the existing analysis cards: specks per frame, channel ranges, flicker size, weave size.

## Dependencies

None. Payload stays at v11 and the installer is untouched. Modern (vs-jetpack) mode already imports havsfunc for home movies, which covers Stab and SMDegrain.

## Phases

| Phase | Contents | Why this order |
|---|---|---|
| 0 | Output clock last; restored-frame preview; config fields; pre-pass scene cuts | Everything else depends on it |
| 1 | Guarded dust removal, dust count, Film dirt page | Highest value, already shown to work |
| 2 | Per-scene colour restore, deflicker, Film colour and Flicker pages | Both use the same pre-pass measurements |
| 3 | SMDegrain film option, CAS, grain at film rate | Lower value or needs visual comparison first |
| 4 | Static defects: detect and report, crop suggestion; research spike on fill | Largest unknown |

Each phase can ship on its own.

## Testing

- **Render tests, not text tests.** One per filter in the style of `home_movie_render.rs`, so a script that cannot make a frame fails the build.
- **"Must survive" cases:** the match flame and the sweater mark on Tony 2a become fixed regression frames for dust removal.
- **Before/after sets** on dust-heavy, fast-motion, low-motion and dark scenes, as the brief lists.
- **Timing** on a full 10 minutes of SD, per filter and together, on an idle machine.

## Open questions

None outstanding. The plan is waiting for approval to start phase 0.
