param([string]$OutputDir)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$assetRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../crates/core/assets/cloud-memes'))
if (!$OutputDir) { $OutputDir = Join-Path $PSScriptRoot '../data/artifacts/cloud-memes-v1' }
$memesOutputRoot = [IO.Path]::GetFullPath($OutputDir)
$manifest = Get-Content -LiteralPath (Join-Path $assetRoot 'manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
New-Item -ItemType Directory -Path $memesOutputRoot -Force | Out-Null
for ($sheetIndex = 0; $sheetIndex -lt $manifest.sheets.Count; $sheetIndex++) {
    $sheetSpec = $manifest.sheets[$sheetIndex]
    $sheet = [Drawing.Bitmap]::FromFile((Join-Path $assetRoot $sheetSpec.file))
    try {
        if ($sheet.Width -ne 2048 -or $sheet.Height -ne 2048 -or $sheetSpec.cells.Count -ne 16) { throw 'Unexpected sticker sheet layout' }
        for ($index = 0; $index -lt 16; $index++) {
            if ($sheetSpec.cells[$index].enabled -eq $false) { continue }
            $category = [string]$sheetSpec.cells[$index].category
            if ($category.IndexOfAny([IO.Path]::GetInvalidFileNameChars()) -ge 0 -or $category -in '.', '..') { throw 'Invalid category' }
            $folder = Join-Path $memesOutputRoot $category
            New-Item -ItemType Directory -Path $folder -Force | Out-Null
            $destination = Join-Path $folder ('cloud-v1-{0:00}-{1:00}.png' -f ($sheetIndex + 1), ($index + 1))
            if (Test-Path -LiteralPath $destination) { continue }
            $x = ($index % 4) * 512 + $manifest.margin
            $y = [Math]::Floor($index / 4) * 512 + $manifest.margin
            $side = 512 - 2 * $manifest.margin
            $rectangle = New-Object Drawing.Rectangle($x, $y, $side, $side)
            $tile = $sheet.Clone($rectangle, $sheet.PixelFormat)
            try { $tile.Save($destination, [Drawing.Imaging.ImageFormat]::Png) }
            finally { $tile.Dispose() }
        }
    } finally { $sheet.Dispose() }
}
Copy-Item -LiteralPath (Join-Path $assetRoot 'manifest.json') -Destination (Join-Path $memesOutputRoot 'manifest.json')
Write-Output $memesOutputRoot
