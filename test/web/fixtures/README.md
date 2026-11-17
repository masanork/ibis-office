`gjs-plex-abm.otf` is a test fixture, not an application font. Its name table
uses the family `GJS Subset`, so the loader can rename that historical label.
The outlines are a subset of the bundled
`crates/ibis-office-app/fonts/IBMPlexSans-Regular.ttf` for `ABm` and keep that
font's license and copyright records. The subsetter in this repository writes
`IBS Subset` and does not load this file at runtime.
