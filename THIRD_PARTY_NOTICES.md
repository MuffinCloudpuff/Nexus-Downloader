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
executables are not committed here. Consult upstream license terms when
distributing a packaged application containing those tools.

Other Rust and npm dependencies retain their respective upstream licenses.
