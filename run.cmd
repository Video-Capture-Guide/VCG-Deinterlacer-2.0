@echo off
rem Build and launch the app from source. A development convenience, not
rem part of the product. Cargo is incremental, so building every time costs
rem about a second when nothing changed and guarantees the window shows the
rem current code.
rem
rem VCG_VSPIPE overrides which vspipe runs, for a machine with more than one
rem VapourSynth where the bundled copy fails VSScript init. The Crop page
rem needs only ffmpeg, so it works either way.

setlocal
set "HERE=%~dp0"
set "EXE=%HERE%target\release\vcg-gui.exe"

echo Building, this takes a few minutes the first time and a second after that...
pushd "%HERE%"
cargo build --release -p vcg-gui || (popd & exit /b 1)
popd

rem Not written as an if(...) block on purpose: inside parentheses cmd expands
rem %PIPVS% when it parses the whole block, which is before the set runs, so
rem the test always saw an empty value and silently skipped this.
if not "%VCG_VSPIPE%"=="" goto :have_vspipe
set "PIPVS=%LOCALAPPDATA%\Programs\Python\Python314\Lib\site-packages\vapoursynth\vspipe.exe"
if exist "%PIPVS%" set "VCG_VSPIPE=%PIPVS%"
:have_vspipe

if not "%VCG_VSPIPE%"=="" echo Using vspipe: %VCG_VSPIPE%
echo Starting %EXE%
start "" "%EXE%"
endlocal
