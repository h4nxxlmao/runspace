# Runspace installer for Windows.
#
#   irm https://raw.githubusercontent.com/h4nxxlmao/runspace/main/install.ps1 | iex
#
# Override the repository with $env:RUNSPACE_REPO, pin a release with
# $env:RUNSPACE_VERSION, or choose a directory with $env:RUNSPACE_INSTALL_DIR.

$ErrorActionPreference = "Stop"

$Repo = if ($env:RUNSPACE_REPO) { $env:RUNSPACE_REPO } else { "h4nxxlmao/runspace" }
$Version = if ($env:RUNSPACE_VERSION) { $env:RUNSPACE_VERSION } else { "latest" }
$InstallDir = if ($env:RUNSPACE_INSTALL_DIR) {
    $env:RUNSPACE_INSTALL_DIR
} else {
    Join-Path $env:LOCALAPPDATA "Runspace\bin"
}

$Arch = $env:PROCESSOR_ARCHITECTURE
if ($Arch -notin @("AMD64", "x86_64")) {
    throw "Runspace currently ships a Windows x64 build; detected architecture '$Arch'."
}

$Asset = "runspace-x86_64-pc-windows-msvc.exe"
if ($Version -eq "latest") {
    $Url = "https://github.com/$Repo/releases/latest/download/$Asset"
} else {
    $Url = "https://github.com/$Repo/releases/download/$Version/$Asset"
}

$TempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("runspace-install-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $TempDir | Out-Null
$Binary = Join-Path $TempDir "runspace.exe"

try {
    Write-Host "Downloading $Asset..."
    Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $Binary
    & $Binary --version | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "the downloaded binary did not run"
    }
} catch {
    Write-Warning "Could not install the prebuilt binary: $($_.Exception.Message)"
    Write-Host "Falling back to a source build with cargo..."
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw "cargo is required for the source fallback. Install Rust from https://rustup.rs and retry."
    }
    $CargoRoot = Join-Path $TempDir "cargo"
    cargo install --git "https://github.com/$Repo.git" runspace --locked --root $CargoRoot
    if ($LASTEXITCODE -ne 0) {
        throw "cargo install failed"
    }
    Copy-Item (Join-Path $CargoRoot "bin\runspace.exe") $Binary -Force
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$Target = Join-Path $InstallDir "runspace.exe"
Copy-Item $Binary $Target -Force
Remove-Item -Recurse -Force $TempDir

Write-Host ""
Write-Host "Installed $Target"
& $Target --version

$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
$parts = @()
if ($UserPath) { $parts = $UserPath -split ';' | Where-Object { $_ } }
if ($parts -notcontains $InstallDir) {
    $newPath = (@($InstallDir) + $parts) -join ';'
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    Write-Host ""
    Write-Host "Added $InstallDir to your user PATH. Open a new terminal to use 'runspace'."
}

Write-Host ""
Write-Host "Native Windows backend: no WSL, no extra downloads."
Write-Host "Run 'runspace doctor' to verify Win32 desktops, job objects and the host session."
Write-Host ""
& $Target doctor
