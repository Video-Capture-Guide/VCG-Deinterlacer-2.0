Download the zip below, unzip it anywhere, and run **VCG Deinterlacer.exe**.

On first run the app downloads the support payload it needs — FFmpeg and a
portable VapourSynth with its plugins, about 158 MiB. That download is checked
against a byte count and a SHA-256 built into the app, and is extracted to a
staging folder and only swapped into place once it is complete, so a failed
download cannot leave you without a working install.

The exe is **not code-signed**, so Windows SmartScreen and Microsoft Defender
will warn about it the first time you run it. The SHA-256 of the zip is
attached as `checksum.txt` if you want to verify what you downloaded.

Requires 64-bit Windows 10 or 11.

See the [README](https://github.com/Video-Capture-Guide/vcg-deinterlacer-rust#readme)
for what the app does and what each setting means.
