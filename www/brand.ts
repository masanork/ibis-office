// SPDX-License-Identifier: AGPL-3.0-only
// Single source of truth for the Web shell's product brand name.
//
// `www/boot.ts` is loaded as a classic `<script src="./boot.js">` (no
// `type="module"`), whereas `www/main.ts` is an ES module. Because `tsc -p www`
// compiles each file standalone without a bundler, declaring the literal as an
// ambient `const enum` lets `boot.ts` reference it via `/// <reference>` so `tsc`
// inlines the string into classic `boot.js` with no `import`/`export` syntax,
// while ES-module entry points import `BRAND_NAME` from `./brand.js`.
declare global {
  const enum WebBrand {
    Name = 'soraneDocs',
  }
}

export const BRAND_NAME = WebBrand.Name;
