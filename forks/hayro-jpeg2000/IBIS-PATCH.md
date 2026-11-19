Upstream source is the crates.io hayro-jpeg2000 0.3.5 release, from
https://github.com/LaurenzV/hayro/tree/a2354aebaf86f5ab13e5fd6b6b0521306488bf9d/hayro-jpeg2000.

The local change handles one LRCP progression-order change (POC) entry per
tile-part when all parts of that tile provide explicit, increasing resolution
bounds over all components and layers. It preserves the default packet iterator
for tiles without POC. The upstream licenses and asset notices are included.

The main header can also override COD with a single LRCP volume covering all
packets. RGN Maxshift is supported for reversible (unquantized) coefficients,
using component defaults from the main header or overrides from the first
tile-part. Main-header and tile-part POC cannot be combined in this subset.
