// SPDX-License-Identifier: AGPL-3.0-only
// Existing editor links such as /?open=... and /?local=1 keep working.
if (location.search) {
  location.replace('./editor.html' + location.search + location.hash);
}
