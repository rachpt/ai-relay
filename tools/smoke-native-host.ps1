# Run in an interactive Windows desktop; does not click Explorer's tray UI.
# Only the process started by this script is inspected, automated, or terminated.
[CmdletBinding()]
param(
    [string]$ExePath,
    [ValidateRange(2, 15)][int]$StepTimeoutSeconds = 8
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) '..\target\release\ai-relay.exe'
}

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class NativeHostSmoke {
    public delegate bool EnumProc(IntPtr window, IntPtr parameter);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    public static extern IntPtr LoadLibraryExW(string file, IntPtr reserved, uint flags);
    [DllImport("kernel32.dll", SetLastError=true)]
    public static extern bool FreeLibrary(IntPtr module);
    [DllImport("kernel32.dll", SetLastError=true)]
    public static extern IntPtr FindResourceW(IntPtr module, IntPtr name, IntPtr type);
    [DllImport("kernel32.dll", SetLastError=true)]
    public static extern uint SizeofResource(IntPtr module, IntPtr resource);
    [DllImport("kernel32.dll", SetLastError=true)]
    public static extern IntPtr LoadResource(IntPtr module, IntPtr resource);
    [DllImport("kernel32.dll")]
    public static extern IntPtr LockResource(IntPtr resource);
    [DllImport("user32.dll", SetLastError=true)]
    public static extern IntPtr LoadImageW(IntPtr module, IntPtr name, uint type, int cx, int cy, uint flags);
    [DllImport("user32.dll")]
    public static extern bool DestroyIcon(IntPtr icon);
    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumProc callback, IntPtr parameter);
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetClassNameW(IntPtr window, StringBuilder text, int capacity);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetWindowTextW(IntPtr window, StringBuilder text, int capacity);
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll", SetLastError=true)]
    public static extern bool PostMessageW(IntPtr window, uint message, UIntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")]
    public static extern IntPtr GetLastActivePopup(IntPtr window);
    [DllImport("user32.dll")]
    public static extern IntPtr GetDlgItem(IntPtr window, int id);
    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();
    public static uint Owner(IntPtr window) {
        uint pid; GetWindowThreadProcessId(window, out pid); return pid;
    }
    public static string ClassName(IntPtr window) {
        var text = new StringBuilder(256); GetClassNameW(window, text, text.Capacity); return text.ToString();
    }
    public static string Title(IntPtr window) {
        var text = new StringBuilder(512); GetWindowTextW(window, text, text.Capacity); return text.ToString();
    }
    public static IntPtr Find(uint pid, string className) {
        IntPtr found = IntPtr.Zero;
        EnumWindows(delegate(IntPtr window, IntPtr parameter) {
            if (Owner(window) == pid && ClassName(window) == className) {
                found = window; return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@

function Wait-Condition([string]$Description, [scriptblock]$Condition) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ($timer.Elapsed.TotalSeconds -lt $StepTimeoutSeconds) {
        if (& $Condition) { return }
        if ($script:child.HasExited) { throw "Process exited early ($($script:child.ExitCode)): $Description" }
        Start-Sleep -Milliseconds 50
    }
    throw "Timed out after ${StepTimeoutSeconds}s: $Description"
}
function Post-Checked([IntPtr]$Window, [uint32]$Message, [UIntPtr]$WParam, [IntPtr]$LParam) {
    if ([NativeHostSmoke]::Owner($Window) -ne $script:child.Id) { throw 'Refusing to message another process.' }
    if (-not [NativeHostSmoke]::PostMessageW($Window, $Message, $WParam, $LParam)) {
        throw "PostMessage failed: $([Runtime.InteropServices.Marshal]::GetLastWin32Error())"
    }
}

$child = $null
$module = [IntPtr]::Zero
$icon = [IntPtr]::Zero
try {
    $exe = Get-Item -LiteralPath (Resolve-Path -LiteralPath $ExePath).Path
    Write-Output ("EXE: {0}`nSIZE: {1} bytes ({2:N2} KiB)" -f $exe.FullName, $exe.Length, ($exe.Length / 1KB))
    # LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE: inspect without executing code.
    $module = [NativeHostSmoke]::LoadLibraryExW($exe.FullName, [IntPtr]::Zero, 0x22)
    if ($module -eq [IntPtr]::Zero) { throw 'LoadLibraryExW resource mapping failed.' }
    $group = [NativeHostSmoke]::FindResourceW($module, [IntPtr]1, [IntPtr]14)
    if ($group -eq [IntPtr]::Zero) { throw 'RT_GROUP_ICON resource ID 1 is missing.' }
    $icon = [NativeHostSmoke]::LoadImageW($module, [IntPtr]1, 1, 0, 0, 0x40)
    if ($icon -eq [IntPtr]::Zero) { throw 'LoadImageW of embedded icon ID 1 failed (no fallback used).' }
    Write-Output 'PASS: embedded icon group ID 1 exists and LoadImageW succeeds without fallback.'
    $resource = [NativeHostSmoke]::FindResourceW($module, [IntPtr]1, [IntPtr]24)
    if ($resource -eq [IntPtr]::Zero) { throw 'RT_MANIFEST resource ID 1 is missing.' }
    $size = [NativeHostSmoke]::SizeofResource($module, $resource)
    if ($size -eq 0 -or $size -gt 1MB) { throw "Invalid manifest size: $size" }
    $loaded = [NativeHostSmoke]::LoadResource($module, $resource)
    if ($loaded -eq [IntPtr]::Zero) { throw 'LoadResource(manifest) failed.' }
    $pointer = [NativeHostSmoke]::LockResource($loaded)
    if ($pointer -eq [IntPtr]::Zero) { throw 'LockResource(manifest) failed.' }
    $bytes = New-Object byte[] $size
    [Runtime.InteropServices.Marshal]::Copy($pointer, $bytes, 0, $bytes.Length)
    $stream = [IO.MemoryStream]::new($bytes, $false)
    $reader = $null
    try {
        $settings = [Xml.XmlReaderSettings]::new()
        $settings.DtdProcessing = [Xml.DtdProcessing]::Prohibit
        $settings.XmlResolver = $null
        $reader = [Xml.XmlReader]::Create($stream, $settings)
        $manifest = [Xml.XmlDocument]::new()
        $manifest.XmlResolver = $null
        $manifest.Load($reader)
    } finally {
        if ($null -ne $reader) { $reader.Dispose() }
        $stream.Dispose()
    }
    if ($manifest.DocumentElement.LocalName -ne 'assembly') { throw 'Manifest has no assembly root.' }
    Write-Output "PASS: embedded manifest ID 1 is valid XML ($size bytes)."
    foreach ($name in @('requestedExecutionLevel', 'dpiAware', 'dpiAwareness', 'longPathAware')) {
        $node = $manifest.SelectSingleNode("//*[local-name()='$name']")
        if ($null -ne $node) { Write-Output ("MANIFEST: " + $node.OuterXml) }
    }
    [void][NativeHostSmoke]::DestroyIcon($icon)
    $icon = [IntPtr]::Zero
    [void][NativeHostSmoke]::FreeLibrary($module)
    $module = [IntPtr]::Zero

    $child = Start-Process -FilePath $exe.FullName -WorkingDirectory $exe.DirectoryName -PassThru
    $script:window = [IntPtr]::Zero
    Wait-Condition 'visible native main window' {
        $script:window = [NativeHostSmoke]::Find([uint32]$child.Id, 'AIRelay.Window.v1')
        $script:window -ne [IntPtr]::Zero -and [NativeHostSmoke]::IsWindowVisible($script:window)
    }
    Write-Output "PASS: native window appeared (PID $($child.Id)); title: $([NativeHostSmoke]::Title($window))"
    Post-Checked $window 0x10 ([UIntPtr]::Zero) ([IntPtr]::Zero) # WM_CLOSE
    $script:noticeDismissed = $false
    Wait-Condition 'first-close notice and hide' {
        $popup = [NativeHostSmoke]::GetLastActivePopup($window)
        if ($popup -ne [IntPtr]::Zero -and $popup -ne $window) {
            Write-Verbose ("Popup: hwnd={0} class={1} owner={2} title={3} IDOK={4}" -f $popup, [NativeHostSmoke]::ClassName($popup), [NativeHostSmoke]::Owner($popup), [NativeHostSmoke]::Title($popup), [NativeHostSmoke]::GetDlgItem($popup, 1))
        }
        if ($popup -ne $window -and $popup -ne [IntPtr]::Zero -and
            [NativeHostSmoke]::Owner($popup) -eq $child.Id -and
            [NativeHostSmoke]::ClassName($popup) -eq '#32770' -and
            [NativeHostSmoke]::IsWindowVisible($popup)) {
            $button = [NativeHostSmoke]::GetDlgItem($popup, 1) # IDOK
            # Some MB_OK dialogs expose their sole button as IDCANCEL (2).
            if ($button -eq [IntPtr]::Zero) { $button = [NativeHostSmoke]::GetDlgItem($popup, 2) }
            if ($button -ne [IntPtr]::Zero) {
                Post-Checked $button 0xF5 ([UIntPtr]::Zero) ([IntPtr]::Zero) # BM_CLICK
                $script:noticeDismissed = $true
            }
        }
        -not [NativeHostSmoke]::IsWindowVisible($window)
    }
    if (-not $noticeDismissed) { throw 'First-close notice was not observed and acknowledged.' }
    Write-Output 'PASS: WM_CLOSE + synthetic notice OK hides the window; process remains alive.'
    # Application-specific V4 notification: ID=1 in HIWORD, WM_LBUTTONDBLCLK in LOWORD.
    Post-Checked $window 0x8001 ([UIntPtr]::Zero) ([IntPtr]0x10203)
    Wait-Condition 'restore through simulated tray callback' { [NativeHostSmoke]::IsWindowVisible($window) }
    Write-Output 'PASS: SIMULATED tray V4 double-click callback restores the window (not a user tray click).'

    # Open the real application menu via simulated WM_CONTEXTMENU notification.
    Post-Checked $window 0x8001 ([UIntPtr]::Zero) ([IntPtr]0x1007B)
    Wait-Condition 'real application popup menu' {
        $menuWindow = [NativeHostSmoke]::Find([uint32]$child.Id, '#32768')
        $menuWindow -ne [IntPtr]::Zero -and [NativeHostSmoke]::IsWindowVisible($menuWindow)
    }
    Write-Output 'PASS: real popup menu appeared after SIMULATED tray context-menu callback.'
    Post-Checked $window 0x1F ([UIntPtr]::Zero) ([IntPtr]::Zero) # WM_CANCELMODE
    Wait-Condition 'popup menu dismissed' {
        $menuWindow = [NativeHostSmoke]::Find([uint32]$child.Id, '#32768')
        $menuWindow -eq [IntPtr]::Zero -or -not [NativeHostSmoke]::IsWindowVisible($menuWindow)
    }
    # Exercise graceful shutdown without pretending this selects the tray Exit command.
    Post-Checked $window 0x16 ([UIntPtr]::new([uint32]1)) ([IntPtr]::Zero) # WM_ENDSESSION
    if (-not $child.WaitForExit($StepTimeoutSeconds * 1000)) { throw 'Session-end shutdown timed out.' }
    if ($child.ExitCode -ne 0) { throw "Exit code was $($child.ExitCode)." }
    Write-Output 'PASS: SIMULATED WM_ENDSESSION causes normal process exit with code 0 (not tray menu Exit).'
    Write-Output 'LIMITS: tray menu Exit selection is NOT verified; synthetic-key selection timed out during development.'
    Write-Output 'LIMITS: no physical Explorer tray clicks, Explorer restart, multi-monitor DPI, or proxy validation.'
} finally {
    if ($icon -ne [IntPtr]::Zero) { [void][NativeHostSmoke]::DestroyIcon($icon) }
    if ($module -ne [IntPtr]::Zero) { [void][NativeHostSmoke]::FreeLibrary($module) }
    if ($null -ne $child) {
        try {
            if (-not $child.HasExited) {
                Write-Output "CLEANUP: terminating only the test-created PID $($child.Id)."
                $child.Kill()
                if (-not $child.WaitForExit(3000)) { throw 'Test-created process did not terminate during cleanup.' }
            }
        } finally { $child.Dispose() }
    }
}
