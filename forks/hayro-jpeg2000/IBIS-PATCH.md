Upstream source is the crates.io hayro-jpeg2000 0.3.5 release, from
https://github.com/LaurenzV/hayro/tree/a2354aebaf86f5ab13e5fd6b6b0521306488bf9d/hayro-jpeg2000.

The local change handles one progression-order change (POC) entry per
tile-part when all parts of that tile provide explicit, increasing resolution
bounds over all components and layers. Each part can use any of the five Part 1 orders. It preserves the default packet iterator
for tiles without POC. The upstream licenses and asset notices are included.

The main header can also override COD with a POC marker with one or more progression volumes. The volumes cover all
layers and components, expand the resolution bound cumulatively, and use any
of the five Part 1 orders. Explicit starting resolutions are supported in the
main header and tile-parts when the ranges leave no gap. RGN Maxshift is supported for reversible (unquantized) coefficients,
using component defaults from the main header or overrides from the first
tile-part. Main-header and tile-part POC cannot be combined in this subset.
