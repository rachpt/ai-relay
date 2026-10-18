# Build a Windows x64 portable PREVIEW with a static C runtime; never include user data.
[CmdletBinding()]
param(
    [string]$CargoPath = 'cargo',
    [ValidatePattern('^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$')]
    [string]$BuildLabel
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$license = Join-Path $root 'LICENSE'
if (-not [IO.File]::Exists($license)) { throw 'Missing project LICENSE; refusing to package.' }
$notices = Join-Path $root 'THIRD-PARTY-NOTICES.txt'
if (-not [IO.File]::Exists($notices)) { throw 'Missing THIRD-PARTY-NOTICES.txt; refusing to package.' }
$cargo = (Get-Command $CargoPath -ErrorAction Stop).Source
$oldFlags = [Environment]::GetEnvironmentVariable('RUSTFLAGS', 'Process')
$oldEncoded = [Environment]::GetEnvironmentVariable('CARGO_ENCODED_RUSTFLAGS', 'Process')
if ($oldEncoded) { throw 'Unset CARGO_ENCODED_RUSTFLAGS before packaging so CRT selection can be verified.' }
function Imports([string]$Path) {
    $data = [IO.File]::ReadAllBytes($Path)
    $pe = [BitConverter]::ToInt32($data, 0x3c)
    if ([BitConverter]::ToUInt32($data,$pe) -ne 0x4550 -or [BitConverter]::ToUInt16($data,$pe+4) -ne 0x8664) { throw 'Expected a Windows x64 PE executable.' }
    $optional = $pe + 24
    if ([BitConverter]::ToUInt16($data,$optional) -ne 0x20b) { throw 'Expected PE32+.' }
    $count = [BitConverter]::ToUInt16($data,$pe+6)
    $section = $optional + [BitConverter]::ToUInt16($data,$pe+20)
    function Offset([uint32]$Rva) {
        for ($i=0; $i -lt $count; $i++) {
            $s = $section + $i*40
            $start = [BitConverter]::ToUInt32($data,$s+12)
            $size = [Math]::Max([BitConverter]::ToUInt32($data,$s+8),[BitConverter]::ToUInt32($data,$s+16))
            if ($Rva -ge $start -and $Rva -lt $start+$size) { return [int]($Rva-$start+[BitConverter]::ToUInt32($data,$s+20)) }
        }
        throw 'Invalid PE import RVA.'
    }
    $rva = [BitConverter]::ToUInt32($data,$optional+120)
    if (-not $rva) { return @() }
    $offset = Offset $rva
    $names = @()
    for ($n=0; $n -lt 512; $n++) {
        $name = [BitConverter]::ToUInt32($data,$offset+12)
        if (-not $name) { return $names }
        $p = Offset $name
        $end = $p
        while ($end -lt $data.Length -and $data[$end] -ne 0 -and $end-$p -lt 260) { $end++ }
        if ($end-$p -ge 260) { throw 'Invalid import name.' }
        $names += [Text.Encoding]::ASCII.GetString($data,$p,$end-$p)
        $offset += 20
    }
    throw 'Import count exceeded safety bound.'
}
Push-Location $root
try {
    # Encoded arguments preserve spaces in resolved home/project paths. Remapping
    # affects embedded compiler paths only, never runtime configuration discovery.
    $flags = @()
    if ($oldFlags) { $flags += @($oldFlags -split '\s+' | Where-Object { $_ }) }
    $flags += @('-C', 'target-feature=+crt-static')
    $homePath = [Environment]::GetFolderPath('UserProfile')
    if ($homePath) { $flags += ('--remap-path-prefix=' + $homePath + '=/build/user') }
    $flags += ('--remap-path-prefix=' + $root + '=/build/ai-relay')
    $env:CARGO_ENCODED_RUSTFLAGS = [string]::Join([char]31, [string[]]$flags)
    & $cargo build --release --locked --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'Portable build failed.' }
    $exe = Join-Path $root 'target\x86_64-pc-windows-msvc\release\ai-relay.exe'
    $imports = @(Imports $exe)
    if ($imports | Where-Object { $_ -match '^(vcruntime|msvcp|concrt|sqlite3|libssl|libcrypto)' }) { throw 'Unexpected separately installed runtime dependency.' }
    $versionMatch = [regex]::Match([IO.File]::ReadAllText((Join-Path $root 'Cargo.toml')), '(?m)^version\s*=\s*"([0-9.]+)"')
    if (-not $versionMatch.Success) { throw 'Cannot determine package version.' }
    $name = 'ai-relay-' + $versionMatch.Groups[1].Value + '-windows-x64-preview'
    if ($BuildLabel) { $name += '-' + $BuildLabel }
    $stage = Join-Path $root ('target\package\' + $name + '-' + [Guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($stage)
    Copy-Item -LiteralPath $exe -Destination (Join-Path $stage 'ai-relay.exe')
    Copy-Item -LiteralPath (Join-Path $root 'docs\PORTABLE.md') -Destination (Join-Path $stage 'README.md')
    Copy-Item -LiteralPath $license -Destination (Join-Path $stage 'LICENSE')
    Copy-Item -LiteralPath $notices -Destination (Join-Path $stage 'THIRD-PARTY-NOTICES.txt')
    [IO.File]::WriteAllLines((Join-Path $stage 'IMPORTS.txt'),$imports,[Text.Encoding]::UTF8)
    $dist = Join-Path $root 'dist'
    [void][IO.Directory]::CreateDirectory($dist)
    $zip = Join-Path $dist ($name + '.zip')
    if ([IO.File]::Exists($zip)) { throw ('Package already exists; refusing to overwrite: ' + $zip) }
    Compress-Archive -LiteralPath @((Join-Path $stage 'ai-relay.exe'),(Join-Path $stage 'README.md'),(Join-Path $stage 'IMPORTS.txt'),(Join-Path $stage 'LICENSE'),(Join-Path $stage 'THIRD-PARTY-NOTICES.txt')) -DestinationPath $zip
    Write-Output ('Preview ZIP: ' + $zip)
    Write-Output ('EXE bytes: ' + (Get-Item -LiteralPath $exe).Length)
    Write-Output ('SHA256: ' + (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash)
    Write-Output ('PE imports: ' + ($imports -join ', '))
    Write-Output 'LIMIT: import inspection is not clean-machine, code-signing, licensing or long-run acceptance.'
} finally {
    [Environment]::SetEnvironmentVariable('RUSTFLAGS',$oldFlags,'Process')
    [Environment]::SetEnvironmentVariable('CARGO_ENCODED_RUSTFLAGS',$oldEncoded,'Process')
    Pop-Location
}
