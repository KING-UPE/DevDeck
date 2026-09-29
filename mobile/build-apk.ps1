<#
    Builds the DevDeck Remote APK.

    RUN THIS AS ADMINISTRATOR the first time.

    Why: Tauri links the compiled Rust library into the Android project with a
    symbolic link, and Windows only lets a non-administrator create those once
    Developer Mode is on. Everything else in the build works without elevation,
    so after the first successful run you can use a normal terminal.

    Right-click PowerShell -> Run as administrator, then:
        cd D:\ME\DevDeck\mobile
        .\build-apk.ps1
#>

$ErrorActionPreference = 'Stop'

function Step($text) { Write-Host "`n==> $text" -ForegroundColor Cyan }
function Ok($text)   { Write-Host "    $text" -ForegroundColor Green }
function Warn($text) { Write-Host "    $text" -ForegroundColor Yellow }

# ---------------------------------------------------------------- toolchain
Step 'Checking the Android toolchain'

$sdk = Join-Path $env:LOCALAPPDATA 'Android\Sdk'
if (-not (Test-Path $sdk)) { throw "Android SDK not found at $sdk" }

$ndk = Get-ChildItem (Join-Path $sdk 'ndk') -Directory -ErrorAction SilentlyContinue |
       Sort-Object Name -Descending | Select-Object -First 1
if (-not $ndk) { throw "No NDK under $sdk\ndk. Install one with sdkmanager." }

$env:ANDROID_HOME     = $sdk
$env:ANDROID_SDK_ROOT = $sdk
$env:NDK_HOME         = $ndk.FullName
$env:ANDROID_NDK_ROOT = $ndk.FullName

# The NDK's clang builds the native dependencies; without it on PATH, crates
# with C code (ring, via rustls) fail with "failed to find tool clang.exe".
$toolchain = Join-Path $ndk.FullName 'toolchains\llvm\prebuilt\windows-x86_64\bin'
if (-not (Test-Path $toolchain)) { throw "NDK toolchain missing at $toolchain" }
$env:PATH = "$toolchain;$env:PATH"

Ok "SDK  $sdk"
Ok "NDK  $($ndk.Name)"

# ------------------------------------------------------------ developer mode
Step 'Checking Developer Mode (needed for symbolic links)'

$key = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock'
$allowed = $false
if (Test-Path $key) {
    $allowed = (Get-ItemProperty $key -Name AllowDevelopmentWithoutDevLicense -ErrorAction SilentlyContinue).AllowDevelopmentWithoutDevLicense -eq 1
}

if ($allowed) {
    Ok 'already on'
} else {
    $elevated = ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()
                ).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    if (-not $elevated) {
        throw @"
Developer Mode is off and turning it on needs administrator.

Re-run this script from an elevated PowerShell, or flip the switch by hand:
  Settings -> System -> For developers -> Developer Mode
"@
    }
    New-Item -Path $key -Force | Out-Null
    New-ItemProperty -Path $key -Name AllowDevelopmentWithoutDevLicense `
                     -PropertyType DWord -Value 1 -Force | Out-Null
    Ok 'enabled'
}

# ------------------------------------------------------------- rust targets
Step 'Checking Rust Android targets'
$targets = (rustup target list --installed) -split "`r?`n"
foreach ($t in 'aarch64-linux-android', 'armv7-linux-androideabi', 'i686-linux-android', 'x86_64-linux-android') {
    if ($targets -notcontains $t) {
        Warn "adding $t"
        rustup target add $t | Out-Null
    }
}
Ok 'present'

# --------------------------------------------------------------------- build
Set-Location $PSScriptRoot

if (-not (Test-Path 'node_modules')) {
    Step 'Installing npm dependencies'
    npm install | Out-Null
}

if (-not (Test-Path 'src-tauri\gen\android')) {
    Step 'Generating the Android project'
    npx tauri android init
}

Step 'Building the APK (several minutes on a cold cache)'
npx tauri android build --apk --debug

# --------------------------------------------------------------------- result
Step 'Result'
$apks = Get-ChildItem 'src-tauri\gen\android\app\build\outputs\apk' -Recurse -Filter *.apk -ErrorAction SilentlyContinue
if (-not $apks) { throw 'The build reported success but produced no APK.' }

foreach ($a in $apks) {
    Ok ("{0}  ({1:N1} MB)" -f $a.FullName, ($a.Length / 1MB))
}

Write-Host @"

Install it on a phone with either:
  adb install -r "<the .apk above>"
or copy the file to the phone and open it (allow install from unknown sources).

This is a debug-signed build: fine for testing, not for the Play Store or for
handing to other people. A public release needs its own signing key.
"@ -ForegroundColor Gray
