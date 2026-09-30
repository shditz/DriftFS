param (
    [string]$Version = "0.1.0"
)

$ErrorActionPreference = "Stop"

Write-Host "Packaging DriftFS v$Version for Windows x86_64..." -ForegroundColor Cyan

$RepoRoot = Split-Path -Parent $PSScriptRoot
$DistDir = Join-Path $RepoRoot "dist"
$TargetRelease = Join-Path $RepoRoot "target\release"
$ExePath = Join-Path $TargetRelease "driftfs-ui.exe"

if (-not (Test-Path $DistDir)) {
    New-Item -ItemType Directory -Path $DistDir | Out-Null
}

if (-not (Test-Path $ExePath)) {
    Write-Host "Building release binary..." -ForegroundColor Yellow
    Push-Location $RepoRoot
    cargo build --release -p driftfs-ui
    Pop-Location
}

# 1. Create portable zip archive
$ZipName = "driftfs-v$Version-windows-x86_64.zip"
$ZipPath = Join-Path $DistDir $ZipName
$TempPackageDir = Join-Path $DistDir "driftfs-v$Version-windows-x86_64"

if (Test-Path $TempPackageDir) {
    Remove-Item -Recurse -Force $TempPackageDir
}
New-Item -ItemType Directory -Path $TempPackageDir | Out-Null

Copy-Item $ExePath $TempPackageDir
Copy-Item (Join-Path $RepoRoot "config.example.toml") $TempPackageDir
Copy-Item (Join-Path $RepoRoot "README.md") $TempPackageDir
Copy-Item (Join-Path $RepoRoot "LICENSE-MIT") $TempPackageDir
Copy-Item (Join-Path $RepoRoot "LICENSE-APACHE") $TempPackageDir

if (Test-Path $ZipPath) {
    Remove-Item -Force $ZipPath
}

Compress-Archive -Path "$TempPackageDir\*" -DestinationPath $ZipPath
Remove-Item -Recurse -Force $TempPackageDir
Write-Host "Created portable archive: $ZipPath" -ForegroundColor Green

# 2. Build Inno Setup installer if compiler is available
$Iscc = Get-Command iscc.exe -ErrorAction SilentlyContinue
if (-not $Iscc) {
    $CommonPaths = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles}\Inno Setup 6\ISCC.exe"
    )
    foreach ($path in $CommonPaths) {
        if (Test-Path $path) {
            $Iscc = $path
            break
        }
    }
}

if ($Iscc) {
    Write-Host "Compiling Inno Setup installer..." -ForegroundColor Yellow
    $IssPath = Join-Path $RepoRoot "installer\windows\driftfs.iss"
    & $Iscc "/DMyAppVersion=$Version" $IssPath
    Write-Host "Created installer in $DistDir" -ForegroundColor Green
} else {
    Write-Host "Inno Setup compiler (ISCC.exe) not found; skipping installer creation." -ForegroundColor Yellow
}

# 3. Generate SHA256 checksums
Push-Location $DistDir
$Checksums = @()
Get-ChildItem -File -Filter "*.zip" | ForEach-Object {
    $hash = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower()
    $Checksums += "$hash  $($_.Name)"
}
Get-ChildItem -File -Filter "*.exe" | ForEach-Object {
    $hash = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower()
    $Checksums += "$hash  $($_.Name)"
}
$Checksums | Out-File -FilePath "SHA256SUMS.txt" -Encoding ascii
Pop-Location

Write-Host "SHA256 checksums written to dist/SHA256SUMS.txt" -ForegroundColor Green
Write-Host "Packaging complete." -ForegroundColor Cyan
