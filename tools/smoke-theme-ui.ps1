# Uses only its own child and temporary config. Never changes Windows theme/high contrast/DPI.
[CmdletBinding()]
param([string]$ExePath, [switch]$CaptureScreenshots)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $ExePath) { $ExePath = Join-Path $PSScriptRoot '..\target\debug\ai-relay.exe' }
$exe = (Get-Item -LiteralPath $ExePath).FullName
# Reuse only the existing PID-checked Win32 helper declaration, not its test body.
$source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'smoke-proxy-ui.ps1'))
$match = [regex]::Match($source, "(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@")
if (-not $match.Success) { throw 'Win32 smoke helper declaration not found' }
if (-not ('ProxyUiSmoke' -as [type])) { Add-Type -TypeDefinition $match.Groups[1].Value }
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class ThemeSmoke {
    [DllImport("user32.dll")] public static extern uint GetGuiResources(IntPtr process, uint flags);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr window, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
}
'@
$temp = Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-theme-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temp)
$config = Join-Path $temp 'config.toml'
[IO.File]::WriteAllText($config, "schema_version = 1`n[proxy]`nauto_start = false`n[ui]`ntheme = 'system'`n")
$child = $null
$window = [IntPtr]::Zero
$primaryError = $null
$cleanupErrors = [Collections.Generic.List[string]]::new()
function Wait-For([string]$Name, [scriptblock]$Test) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ($timer.Elapsed.TotalSeconds -lt 15) {
        if ($child.HasExited) { throw "Child exited: $Name (exit code $($child.ExitCode))" }
        if (& $Test) { return }
        Start-Sleep -Milliseconds 50
    }
    throw "Timeout: $Name"
}
function Control([int]$Id) { [ProxyUiSmoke]::GetDlgItem($window, $Id) }
function Send([IntPtr]$Handle, [uint32]$Message, [uint32]$Value) { [ProxyUiSmoke]::Send($Handle, [uint32]$child.Id, $Message, $Value) }
function Start-Gui {
    $script:child = Start-Process -FilePath $exe -ArgumentList @('--config-dir', ('"' + $temp + '"')) -PassThru
    Wait-For 'window and loaded config' {
        $script:window = [ProxyUiSmoke]::Find([uint32]$child.Id)
        if ($window -eq [IntPtr]::Zero) { return $false }
        # v6 controls can initially be enabled during creation, before the first
        # worker event populates configuration. Wait for the actual status event.
        $loaded = @([ProxyUiSmoke]::Children($window, 'Static') | Where-Object {
            [ProxyUiSmoke]::Title($_).StartsWith(([string][char]0x4EE3 + [char]0x7406 + [char]0x72B6 + [char]0x6001 + [char]0xFF1A))
        }).Count -gt 0
        $loaded -and [ProxyUiSmoke]::IsWindowEnabled((Control 2001)) -and [ProxyUiSmoke]::IsWindowEnabled((Control 2030))
    }
    [ProxyUiSmoke]::SelectTab($window, [uint32]$child.Id, 3)
    $commonControls = @($child.Modules | Where-Object {
        $_.ModuleName -ieq 'comctl32.dll' -and $_.FileVersionInfo.FileMajorPart -eq 6
    })
    if ($commonControls.Count -eq 0) { throw 'Test child did not load Common Controls v6' }
    Write-Output "Common Controls v6 loaded: $($commonControls[0].FileName)"
}
function Stop-Gui {
    [ProxyUiSmoke]::Post($window, [uint32]$child.Id, 0x111, 1002)
    if (-not $child.WaitForExit(38000)) { throw 'Graceful exit timeout' }
    if ($child.ExitCode -ne 0) { throw "Exit code $($child.ExitCode)" }
    $child.Dispose()
    $script:child = $null
}
function Category([uint32]$Index) {
    Send (Control 2039) 0x14E $Index
    Send $window 0x111 (2039 -bor (1 -shl 16))
}
function Choose([uint32]$Index) {
    Send (Control 2030) 0x14E $Index # CB_SETCURSEL does not generate selection notification.
    Send $window 0x111 (2030 -bor (1 -shl 16)) # CBN_SELCHANGE, no cross-process pointers.
    Send $window 0x400 0 # Drain queued refresh before assertions.
}
function Capture([string]$Name) {
    if (-not $CaptureScreenshots) { return }
    Add-Type -AssemblyName System.Drawing
    $rect = New-Object ThemeSmoke+Rect
    if (-not [ThemeSmoke]::GetWindowRect($window, [ref]$rect)) { throw 'Window rectangle unavailable' }
    $bitmap = [Drawing.Bitmap]::new($rect.Right - $rect.Left, $rect.Bottom - $rect.Top)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $dc = $graphics.GetHdc()
        try { if (-not [ThemeSmoke]::PrintWindow($window, $dc, 2)) { throw 'PrintWindow failed; no visual validation' } }
        finally { $graphics.ReleaseHdc($dc) }
        $directory = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\target\theme-smoke'))
        [void][IO.Directory]::CreateDirectory($directory)
        $bitmap.Save((Join-Path $directory ($Name + '.png')))
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
}
try {
    Start-Gui
    $original = [IO.File]::ReadAllText($config)
    Wait-For 'settings page visible' { [ProxyUiSmoke]::IsWindowVisible((Control 2001)) }
    $network = @([ProxyUiSmoke]::Children($window, 'Edit') | Where-Object { [ProxyUiSmoke]::IsWindowVisible($_) })
    if ($network.Count -ne 4) { throw "Expected 4 visible settings edits, got $($network.Count)" }
    $draft = 'http://127.0.0.1:1/unsaved-theme-test'
    [ProxyUiSmoke]::SetText($network[0], [uint32]$child.Id, $draft)
    if ([ProxyUiSmoke]::ReadText($network[0], [uint32]$child.Id) -ne $draft) { throw 'Network draft did not reach settings edit' }
    Category 5
    Wait-For 'theme category visible' { [ProxyUiSmoke]::IsWindowVisible((Control 2030)) }
    $baseline = [ThemeSmoke]::GetGuiResources($child.Handle, 0)
    foreach ($index in @(1,2,0,2)) {
        Choose $index
        foreach ($page in @(0,1,2,3)) {
            [ProxyUiSmoke]::SelectTab($window, [uint32]$child.Id, $page)
            # Simulated notifications only; no system preference mutation.
            Send $window 0x1A 0
            Send $window 0x31A 0
            Start-Sleep -Milliseconds 80
            if ($page -eq 3) {
                foreach ($category in @(0,1,2,3,4,5)) {
                    Category $category
                    Capture ("mode-$index-settings-$category")
                }
            } else { Capture ("mode-$index-page-$page") }
        }
        if ([ProxyUiSmoke]::ReadText($network[0], [uint32]$child.Id) -ne $draft) { throw 'Theme refresh overwrote unsaved input' }
        if ([IO.File]::ReadAllText($config) -cne $original) { throw 'Theme preview wrote config' }
    }
    for ($i = 0; $i -lt 60; $i++) { Choose ($i % 3) }
    Choose 2
    Start-Sleep -Milliseconds 200
    $after = [ThemeSmoke]::GetGuiResources($child.Handle, 0)
    if ($after -gt $baseline + 8) { throw "GDI object growth: $baseline -> $after" }
    [ProxyUiSmoke]::Post((Control 2001), [uint32]$child.Id, 0xF5, 0)
    Wait-For 'normal worker save persists theme and network draft' {
        $saved = [IO.File]::ReadAllText($config)
        [ProxyUiSmoke]::IsWindowEnabled((Control 2001)) -and $saved.Contains('theme = "dark"') -and $saved.Contains($draft)
    }
    # SaveAndApply now starts valid saved configs even when auto_start=false.
    [ProxyUiSmoke]::Post((Control 2003), [uint32]$child.Id, 0xF5, 0)
    Wait-For 'stop before reload' { [ProxyUiSmoke]::IsWindowEnabled((Control 2004)) }
    Choose 1
    [ProxyUiSmoke]::Post((Control 2004), [uint32]$child.Id, 0xF5, 0)
    Wait-For 'reload enabled' { [ProxyUiSmoke]::IsWindowEnabled((Control 2004)) }
    Start-Sleep -Milliseconds 250
    $darkCaption = [string][char]0x6DF1 + [char]0x8272
    Wait-For 'reload restores persisted Dark selection' { [ProxyUiSmoke]::ReadText((Control 2030), [uint32]$child.Id) -eq $darkCaption }
    Stop-Gui
    Start-Gui
    # Assert actual native combo selection after a fresh process loads the saved config.
    $caption = [ProxyUiSmoke]::ReadText((Control 2030), [uint32]$child.Id)
    if ($caption -ne ([string][char]0x6DF1 + [char]0x8272)) { throw "Restart did not restore Dark selection: $caption" }
    $actualDpi = [ThemeSmoke]::GetDpiForWindow($window)
    Stop-Gui
} catch {
    # Save the original ErrorRecord (including invocation/stack information).
    # Cleanup below must not replace the test failure with a sharing violation.
    $primaryError = $_
} finally {
    $exited = $null -eq $child
    if ($null -ne $child) {
        try {
            if (-not $child.HasExited) {
                try {
                    if ($window -ne [IntPtr]::Zero) {
                        [ProxyUiSmoke]::Post($window, [uint32]$child.Id, 0x111, 1002)
                    }
                } catch {
                    Write-Warning "Could not request test-child exit: $($_.Exception.Message)"
                }
                if (-not $child.WaitForExit(38000)) {
                    # Kill through the exact Process object we created, never by name
                    # or a fresh PID lookup that could target an unrelated process.
                    $child.Kill()
                    if (-not $child.WaitForExit(10000)) { throw 'Test child did not exit after termination' }
                }
            }
            $exited = $child.HasExited
        } catch {
            $cleanupErrors.Add("Test-child shutdown failed: $($_.Exception.Message)")
        } finally {
            $child.Dispose()
            $script:child = $null
        }
    }
    if ($exited) {
        # Antivirus/indexers can briefly retain handles after process exit.
        # Retry only our unique test directory, with a bounded total delay.
        for ($attempt = 0; $attempt -lt 20; $attempt++) {
            try {
                if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
                break
            } catch {
                if ($attempt -eq 19) {
                    $cleanupErrors.Add("Temporary directory retained at ${temp}: $($_.Exception.Message)")
                } else { Start-Sleep -Milliseconds 250 }
            }
        }
    } else {
        $cleanupErrors.Add("Temporary directory retained because test-child exit was not confirmed: $temp")
    }
}
if ($null -ne $primaryError) {
    foreach ($cleanupError in $cleanupErrors) { Write-Warning $cleanupError }
    $PSCmdlet.ThrowTerminatingError($primaryError)
}
if ($cleanupErrors.Count -gt 0) { throw ($cleanupErrors -join [Environment]::NewLine) }
Write-Output "PASS: preview/no implicit I/O, all four tabs, setting/theme notifications, draft retention, worker save, reload, restart, child exit and cleanup; GDI $baseline -> $after. Actual DPI: $actualDpi."
Write-Output 'LIMIT: synthetic input, one desktop DPI only; no real high-contrast toggle, physical keyboard, multi-monitor or screen-reader validation. Screenshots use PrintWindow, not physical desktop presentation.'
