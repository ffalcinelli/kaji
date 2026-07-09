$ErrorActionPreference = "Stop"
$GitHubRepo = "ffalcinelli/kaji"

Write-Host "Installing kaji..." -ForegroundColor Cyan

# kaji releases x86_64 for windows
$Target = "x86_64-pc-windows-msvc"

$InstallDir = Join-Path $HOME ".local\bin"
if (-Not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
}

$DownloadUrl = "https://github.com/$GitHubRepo/releases/latest/download/kaji-${Target}.zip"
$TempDir = Join-Path $env:TEMP "kaji-install-$(New-Guid)"
New-Item -ItemType Directory -Force -Path $TempDir | Out-Null
$ZipFile = Join-Path $TempDir "kaji.zip"

Write-Host "Downloading $DownloadUrl..."
Invoke-WebRequest -Uri $DownloadUrl -OutFile $ZipFile

Write-Host "Extracting..."
Expand-Archive -Path $ZipFile -DestinationPath $TempDir -Force

$KajiBin = Get-ChildItem -Path $TempDir -Recurse -Filter "kaji.exe" | Select-Object -First 1
if (-Not $KajiBin) {
    Write-Host "Error: kaji.exe not found in archive." -ForegroundColor Red
    Remove-Item -Path $TempDir -Recurse -Force
    exit 1
}

Move-Item -Path $KajiBin.FullName -Destination (Join-Path $InstallDir "kaji.exe") -Force
Remove-Item -Path $TempDir -Recurse -Force

Write-Host "Successfully installed kaji to $InstallDir\kaji.exe" -ForegroundColor Green

# Check Path
$UserPath = [Environment]::GetEnvironmentVariable("PATH", "User")
if ($UserPath -notmatch [regex]::Escape($InstallDir)) {
    Write-Host "`nWARNING: The installation directory ($InstallDir) is not in your PATH." -ForegroundColor Yellow
    Write-Host "To add it to your current session, run:"
    Write-Host "  `$env:PATH += `";$InstallDir`""
    Write-Host "To add it permanently, add it to your User PATH via Windows Settings."
}
