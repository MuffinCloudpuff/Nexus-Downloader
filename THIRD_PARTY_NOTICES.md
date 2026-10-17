# Third-party notices

## Cat Catch

The extension reuses media-capture and recorder scripts from
[xifangczy/cat-catch](https://github.com/xifangczy/cat-catch). Source is pinned as
the `browser-extension/upstream` submodule at commit
`3dd20ab5b7ebe64feee442fc677f81858394839e`.

Cat Catch is distributed under the GNU General Public License v3. Preserve its
license and applicable notices when redistributing its code or derived portions.
The license is in `browser-extension/upstream/LICENSE` after initialization.
Local adaptations live in `browser-extension/app/public/catch-script/`.

## Historical project provenance

The development checkout previously included the Ghost-Downloader-3 Python
project. That project and its old application/UI are not included in this source
snapshot. Existing adaptations and design history retain their provenance;
cleanup is not a relicensing of any derived code.

The existing Ghost-Downloader-3 GPL v3 license text is retained at
`third-party/Ghost-Downloader-3-LICENSE` for applicable derived portions. No
third-party copyright or license notice is removed by the directory cleanup.

## Media tools

yt-dlp, FFmpeg, and Deno are external tools with their own licenses. Their
executables are not committed to this source repository.

The v0.1.0 Windows portable package contains unmodified FFmpeg / ffprobe 8.1
essentials binaries from Gyan (GPL v3), plus Deno 2.8.2 (MIT). The package retains
their license texts, the FFmpeg build configuration/library versions, and source
links under `licenses/`. These tools run as separate processes.

- FFmpeg binary distribution: https://github.com/GyanD/codexffmpeg/releases/tag/8.1
- Exact FFmpeg source: https://github.com/FFmpeg/FFmpeg/commit/9047fa1b08
- Build provider and source/build references: https://www.gyan.dev/ffmpeg/builds/
- Deno source and license: https://github.com/denoland/deno/tree/v2.8.2

yt-dlp is downloaded and verified on demand, rather than copied from a user's
installation. Its source and licensing are at https://github.com/yt-dlp/yt-dlp.

The accompanying Nexus source asset includes the pinned Cat Catch submodule;
`npm ci` and the checked-in build scripts reproduce the extension. It does not
include the abandoned Ghost Python application. Retained license notices are
not executable application components and must not be removed.

Other Rust and npm dependencies retain their respective upstream licenses.
