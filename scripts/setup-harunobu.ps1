# Install the fixed Harunobu reference runtime locally on Windows.
$ErrorActionPreference = 'Stop'
$commit = '88014ff9cfe685d4e0e5f24a2d4d2e021ca3fd9f'
if (-not $env:LOCALAPPDATA) { throw 'LOCALAPPDATA is not set' }
$root = Join-Path $env:LOCALAPPDATA 'Sorane Docs\harunobu'
foreach ($name in @('git', 'uv')) {
    if (-not (Get-Command $name -ErrorAction SilentlyContinue)) { throw "$name is required" }
}
if (Test-Path -LiteralPath $root) {
    if (-not (Test-Path -LiteralPath (Join-Path $root '.git'))) { throw "Existing runtime is not a Git checkout: $root" }
    $head = (& git -C $root rev-parse HEAD 2>$null)
    if ($LASTEXITCODE -ne 0 -or $head -ne $commit) { throw "Existing runtime has a different revision: $root" }
    $status = @(& git -C $root status --porcelain --untracked-files=all)
    if ($LASTEXITCODE -ne 0) { throw "Cannot inspect runtime: $root" }
    $unexpected = @($status | Where-Object { $_ -ne '?? .ibis-harunobu-commit' })
    if ($unexpected.Count -ne 0) { throw "Existing runtime is modified: $root" }
} else {
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $root) | Out-Null
    & git clone --no-checkout https://github.com/digital-go-jp/machine_readability_rule.git $root
    if ($LASTEXITCODE -ne 0) { throw 'Harunobu clone failed' }
    & git -C $root checkout --detach $commit
    if ($LASTEXITCODE -ne 0) { throw 'Harunobu checkout failed' }
}
& uv sync --frozen --no-dev --directory $root
if ($LASTEXITCODE -ne 0) { throw 'Harunobu environment installation failed' }
# No BOM: the application compares the marker byte-for-byte after trimming.
[System.IO.File]::WriteAllText((Join-Path $root '.ibis-harunobu-commit'), "$commit`n", [System.Text.UTF8Encoding]::new($false))
Write-Host "Harunobu $commit installed at $root"
