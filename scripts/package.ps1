#Requires -Version 5.1
<#
.SYNOPSIS
    Build and package buna-shimeji into a distributable win64 zip.

.DESCRIPTION
    Automates the manual steps documented in doc/releasing.md (§1 構成 / §3 パッケージ).
    リポジトリルートを $PSScriptRoot から解決するため、どのカレントディレクトリから
    実行しても動作する。書き込み・削除は -OutputDir（既定 target/package）配下のみ。

.PARAMETER Version
    パッケージのバージョン文字列（例: 0.1.0）。省略時は Cargo.toml の version を使用。

.PARAMETER SkipBuild
    指定時は cargo build --release をスキップする。

.PARAMETER OutputDir
    出力先ディレクトリ。省略時は <repo>/target/package。相対指定はリポジトリルート基準。

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package.ps1
.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package.ps1 -SkipBuild -Version 0.1.0
#>
[CmdletBinding()]
param(
    [Parameter()]
    [string]$Version,

    [Parameter()]
    [switch]$SkipBuild,

    [Parameter()]
    [string]$OutputDir
)

$ErrorActionPreference = 'Stop'

# エラー時は英語メッセージで明示的に非ゼロ終了する（Stop の例外に依存しない）
function Fail {
    param([Parameter(Mandatory = $true)][string]$Message)
    [Console]::Error.WriteLine("ERROR: $Message")
    exit 1
}

# --- パス解決（カレントディレクトリ非依存） ---------------------------------
$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path

if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $RepoRoot 'target\package'
}
elseif (-not [System.IO.Path]::IsPathRooted($OutputDir)) {
    $OutputDir = Join-Path $RepoRoot $OutputDir
}
$OutputDir = [System.IO.Path]::GetFullPath($OutputDir)

$CargoToml       = Join-Path $RepoRoot 'Cargo.toml'
$ExePath         = Join-Path $RepoRoot 'target\release\shimeji.exe'
$ConfSrc         = Join-Path $RepoRoot 'conf'
$ImgSrc          = Join-Path $RepoRoot 'img'
$ReadmeSrc       = Join-Path $RepoRoot 'README.md'
$LicenseSrc      = Join-Path $RepoRoot 'LICENSE'
$DefaultSettings = Join-Path $ConfSrc 'settings.default.toml'

# --- 事前チェック -----------------------------------------------------------
if (-not (Test-Path -LiteralPath $CargoToml -PathType Leaf)) {
    Fail "Cargo.toml not found: $CargoToml"
}
if (-not (Test-Path -LiteralPath $DefaultSettings -PathType Leaf)) {
    Fail "Default settings template not found: $DefaultSettings"
}

# --- バージョン決定 ---------------------------------------------------------
if ([string]::IsNullOrWhiteSpace($Version)) {
    $cargoText = Get-Content -LiteralPath $CargoToml -Raw
    $match = [regex]::Match($cargoText, '(?m)^\s*version\s*=\s*"([^"]+)"')
    if (-not $match.Success) {
        Fail "Could not parse 'version' from Cargo.toml: $CargoToml"
    }
    $Version = $match.Groups[1].Value
}
Write-Host "Packaging buna-shimeji v$Version"

# --- ビルド -----------------------------------------------------------------
if (-not $SkipBuild) {
    Write-Host "Building release binary (cargo build --release)..."
    Push-Location -LiteralPath $RepoRoot
    try {
        & cargo build --release
        if ($LASTEXITCODE -ne 0) {
            Fail "cargo build --release failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        Pop-Location
    }
}

if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
    Fail "Release binary not found: $ExePath (build it first or omit -SkipBuild)"
}

# --- ステージ準備 -----------------------------------------------------------
$StageName = "buna-shimeji-v$Version-win64"
$StageDir  = Join-Path $OutputDir $StageName
$ZipPath   = Join-Path $OutputDir "$StageName.zip"

if (-not (Test-Path -LiteralPath $OutputDir -PathType Container)) {
    New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
}

# ステージは毎回作り直す（-OutputDir 配下のみを対象にする）
if (Test-Path -LiteralPath $StageDir) {
    Remove-Item -LiteralPath $StageDir -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $StageDir | Out-Null

# --- コピー -----------------------------------------------------------------
Write-Host "Staging files into $StageDir"
Copy-Item -LiteralPath $ExePath   -Destination $StageDir
Copy-Item -LiteralPath $ConfSrc   -Destination (Join-Path $StageDir 'conf') -Recurse
Copy-Item -LiteralPath $ImgSrc    -Destination (Join-Path $StageDir 'img')  -Recurse
Copy-Item -LiteralPath $ReadmeSrc -Destination $StageDir
Copy-Item -LiteralPath $LicenseSrc -Destination $StageDir

# --- settings テンプレート差し替え -----------------------------------------
# 開発環境の settings.toml が混入していれば除去し、default テンプレートを
# settings.toml として同封する（doc/releasing.md §1）。
$StageConf        = Join-Path $StageDir 'conf'
$StageSettings    = Join-Path $StageConf 'settings.toml'
$StageDefaultPath = Join-Path $StageConf 'settings.default.toml'

Remove-Item -LiteralPath $StageSettings -Force -ErrorAction Ignore

if (-not (Test-Path -LiteralPath $StageDefaultPath -PathType Leaf)) {
    Fail "Settings template missing in stage: $StageDefaultPath"
}
Rename-Item -LiteralPath $StageDefaultPath -NewName 'settings.toml'

# ステージ内に default テンプレートが残っておらず、settings.toml が
# テンプレートと同一内容であることを検証する。
if (Test-Path -LiteralPath $StageDefaultPath) {
    Fail "settings.default.toml must not remain in the package: $StageDefaultPath"
}
if (-not (Test-Path -LiteralPath $StageSettings -PathType Leaf)) {
    Fail "Packaged settings.toml not found: $StageSettings"
}

$templateHash = (Get-FileHash -LiteralPath $DefaultSettings -Algorithm SHA256).Hash
$stageHash    = (Get-FileHash -LiteralPath $StageSettings   -Algorithm SHA256).Hash
if ($templateHash -ne $stageHash) {
    Fail "Packaged conf/settings.toml does not match conf/settings.default.toml"
}

# --- zip 作成 ---------------------------------------------------------------
Write-Host "Compressing archive..."
Compress-Archive -Path $StageDir -DestinationPath $ZipPath -Force

# --- 結果表示 ---------------------------------------------------------------
$zipInfo = Get-Item -LiteralPath $ZipPath
$zipHash = (Get-FileHash -LiteralPath $ZipPath -Algorithm SHA256).Hash

Write-Host ""
Write-Host "Package created successfully:"
Write-Host "  Path   : $($zipInfo.FullName)"
Write-Host "  Size   : $($zipInfo.Length) bytes"
Write-Host "  SHA256 : $zipHash"

exit 0
