# Third-party notices

Strata includes or derives assets from the following projects.

## Windows GTK runtime

- Project: <https://www.gtk.org/>
- Version: GTK 4.24.1, with MSYS2 font hinting and Strata's Windows IME patch
- License: LGPL-2.1-or-later; `gtkimcontextime.c` retains its LGPL-2.0-or-later notice
- Original IME authors: Takuro Ashie (2003), Kazuki IWAMOTO (2003–2004)
- Source, checksums, changes and build instructions: [`packaging/windows/runtime/`](packaging/windows/runtime/README.md)

Windows release bundles include corresponding source, patches, licenses and DLL
identities under `share/strata/runtime/`. They also include GstPlay 1.28.8 with
a version-specific worker-disposal patch derived from `packaging/media-runtime/`.
GstPlay retains its LGPL-2.0-or-later notice and credits Sebastian Dröge
(2014–2015), Brijesh Singh (2015), Stephan Hesse (2019–2020), and Philippe Normand
(2020). Its source and build instructions are documented with the Windows runtime.

## MathJax

- Projects: <https://github.com/mathjax/MathJax-src>, <https://github.com/mathjax/MathJax-fonts>
- Version: MathJax and MathJax TeX SVG font data 4.1.3
- Copyright: 2017–2026 The MathJax Consortium
- License: Apache License 2.0
- Included asset: `data/math-renderer.js` (bundled/minified base and AMS renderer with TeX SVG glyph data)
- Full license: [`data/licenses/MathJax-Apache-2.0.txt`](data/licenses/MathJax-Apache-2.0.txt)

The bundle adds Strata's bounded offline rendering entry point. Build inputs and
regeneration instructions are in [`packaging/math-renderer/`](packaging/math-renderer/README.md).

## JetBrains Mono

- Project: <https://github.com/JetBrains/JetBrainsMono>
- Version: 2.304
- Copyright: JetBrains s.r.o.
- License: SIL Open Font License 1.1
- Included asset: `data/fonts/JetBrainsMono[wght].ttf`
- Full license: [`data/licenses/JetBrainsMono-OFL-1.1.txt`](data/licenses/JetBrainsMono-OFL-1.1.txt)

The font is distributed unmodified. Strata materializes the embedded font in its private cache at runtime so it is available without changing the user's system font installation.

## Lucide

- Project: <https://github.com/lucide-icons/lucide>
- Version: 1.35.0
- Copyright: Lucide Contributors and Feather Icons contributors
- License: ISC
- Included assets: curated and namespaced SVG icons under `data/icons/scalable/actions/`
- Full license: [`data/licenses/Lucide-ISC.txt`](data/licenses/Lucide-ISC.txt)

The SVGs retain Lucide geometry. The table sort caret uses the Lucide triangle with a filled foreground, rotated by GTK for descending order. Their foreground color was changed from `currentColor` to GTK's symbolic foreground color so GTK can recolor them according to the active theme.

## Catppuccin VSCode Icons

- Project: <https://github.com/catppuccin/vscode-icons>
- Revision: `b6915da9f6889b683a110aa747de96c2820a537d` (v1.26.0)
- Copyright: 2023 Catppuccin, 2023 thang-nm
- License: MIT
- Included assets: file-type icons under `data/icons/catppuccin/strata-ctp-*.svg`, copied from the upstream `icons/css-variables/` set, and the filename and extension associations in `src/ui/browser/entry/catppuccin.rs`, both generated from upstream by `scripts/catppuccin_icons.py`
- Full license: [`data/licenses/Catppuccin-VSCode-Icons-MIT.txt`](data/licenses/Catppuccin-VSCode-Icons-MIT.txt)

The SVG geometry is unchanged. Each `var(--vscode-ctp-*)` color is resolved at render time to the Catppuccin Mocha or Latte palette depending on the active theme's background, and strokes are scaled with the same weight factor as the Lucide icons.

## Tinted Theming schemes

- Project: <https://github.com/tinted-theming/schemes>
- Revision: `fdca32a0d14ec80ad83a78a9ccb85592ca6cb9e1`
- Copyright: Tinted Theming and the scheme authors identified by the upstream files
- License: MIT
- Derived asset: `data/themes/catalog.toml`
- Full license: [`data/licenses/Tinted-Theming-MIT.txt`](data/licenses/Tinted-Theming-MIT.txt)

The bundled catalog contains curated Base16 palettes derived from this project, with a few foreground values adjusted for readability (noted in the catalog), alongside Strata's original themes. The Base16 mapping to Strata's semantic UI tokens is documented in the catalog.

## GTK and GStreamer media-runtime patch kit

- Projects: <https://gitlab.gnome.org/GNOME/gtk> and <https://gitlab.freedesktop.org/gstreamer/gstreamer>
- Source versions: GTK 4.22.4 and gst-plugins-bad 1.28.6
- License of the patched source files: LGPL-2.0-or-later (GTK's project license is LGPL-2.1-or-later)
- Included material: source-context patches under `packaging/media-runtime/patches/`
- Full licenses: [GTK](packaging/media-runtime/licenses/GTK-COPYING) and [GStreamer](packaging/media-runtime/licenses/GSTREAMER-COPYING)

The experimental patch kit preserves upstream source notices and identifies pinned
source archives. It does not bundle toolkit binaries or alter Strata's release
artifacts. See its [build and redistribution requirements](packaging/media-runtime/README.md).

## UnRAR

- Project: <https://www.rarlab.com/rar_add.htm>
- Bundled source: UnRAR 7.01 (2024-05-12), statically linked by `unrar_sys` 0.5.8 only with the default-enabled `rar` Cargo feature
- Copyright: Alexander Roshal
- License: custom UnRAR license; **not** MIT or Apache-2.0
- Full license: [`data/licenses/UnRAR.txt`](data/licenses/UnRAR.txt), also shipped as `UnRAR.txt` in release archives

UnRAR permits RAR extraction but prohibits using its source to develop a
RAR-compatible archiver or recreate the proprietary RAR compression algorithm.
The wrapper crates' permissive metadata does not cover this native implementation.
The pinned crate archive has SHA-256
`8b77675b883cfbe6bf41e6b7a5cd6008e0a83ba497de3d96e41a064bbeead765`.
Strata does not modify its vendored source. Builds using `--no-default-features`
without `--features rar` exclude both wrapper crates and this native implementation.
The source distribution retains this notice for builds that enable RAR support.

## Rust dependencies

Rust dependency licenses are declared in each package's metadata and are validated in CI with `cargo-deny`. To review the current dependency graph locally, run:

```bash
cargo deny check licenses
```

### arrayref

The bundled icon renderer uses `arrayref` 0.3.9 through `resvg` / `tiny-skia`.
Its BSD-2-Clause license requires the following notice in binary distributions.
This document is included in release archives and installed license materials.

Source: <https://github.com/droundy/arrayref>

```text
Copyright (c) 2015 David Roundy <roundyd@physics.oregonstate.edu>
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

1. Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the
   distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### kamadak-exif

RAW photo metadata uses `kamadak-exif` 0.6.1 under the BSD-2-Clause license.

Source: <https://github.com/kamadak/exif-rs>

```text
Copyright (c) 2016-2023 KAMADA Ken'ichi.
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:
1. Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE AUTHOR AND CONTRIBUTORS ``AS IS'' AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
ARE DISCLAIMED.  IN NO EVENT SHALL THE AUTHOR OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS
OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION)
HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT
LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY
OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF
SUCH DAMAGE.
```

### mutate_once

`kamadak-exif` depends on `mutate_once` 0.1.2 under the BSD-2-Clause license.

Source: <https://github.com/kamadak/mutate_once-rs>

```text
Copyright (c) 2019 KAMADA Ken'ichi.
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:
1. Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE AUTHOR AND CONTRIBUTORS ``AS IS'' AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
ARE DISCLAIMED.  IN NO EVENT SHALL THE AUTHOR OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS
OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION)
HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT
LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY
OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF
SUCH DAMAGE.
```

## HatchZip core

Space-saving extraction on Windows uses `hatchzip-core` 0.1.4 from
[HatchZip](https://github.com/xevrion/hatchzip), reviewed at upstream revision
`d6a4126435cd6a1ed2d158c138d5afe25d131406`. It is used under its MIT license.
The optional UnRAR engine in HatchZip is not enabled.

```text
MIT License

Copyright (c) 2026 Yash Bavadiya

Permission is hereby granted, free of charge, to any
person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the
Software without restriction, including without
limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software
is furnished to do so, subject to the following
conditions:

The above copyright notice and this permission notice
shall be included in all copies or substantial portions
of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

## macOS Classic themes

The Light and Dark palettes are adapted from [huacnlee/zed-theme-macos-classic](https://github.com/huacnlee/zed-theme-macos-classic), revision `22266e11c4b8962be2e259011251b4016ff00552`.

```text
MIT License

Copyright (c) 2023 Jason Lee.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Material Icon Theme

The owner-requested optional Material icon collection is imported unchanged from
[zed-extensions/material-icon-theme](https://github.com/zed-extensions/material-icon-theme), revision `95cc434119c669e0ff65a6bccfd026ec70e16b63`.
It includes all 1,252 SVGs and the upstream filename, suffix and directory mappings.
The SVGs originate from Material Extensions `material-icon-theme` 5.39.0 (MIT).
The Zed manifest and importer source are Apache-2.0; the complete notices follow.

```text
The MIT License (MIT)
Copyright (c) 2025 Material Extensions

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

```text
                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

      To apply the Apache License to your work, attach the following
      boilerplate notice, with the fields enclosed by brackets "[]"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Copyright [yyyy] [name of copyright owner]

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
```
