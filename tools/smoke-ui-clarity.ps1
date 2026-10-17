# Windows PowerShell 5.1 / interactive desktop. Local mock and our exact child only.
# This is chart/theme validation of an isolated build, NOT main history layout acceptance.
[CmdletBinding()]
param([string]$ExePath)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $PSScriptRoot '..\target\ui-clarity-validation\target\debug\ai-relay.exe'
}
$exe = (Get-Item -LiteralPath $ExePath).FullName
# Extract declarations only: never dot-source or execute the existing smoke test.
$source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'smoke-proxy-ui.ps1'))
$declaration = [regex]::Match($source, "(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@")
if (-not $declaration.Success) { throw 'Missing Win32 smoke helper declaration' }
if (-not ('ProxyUiSmoke' -as [type])) { Add-Type -TypeDefinition $declaration.Groups[1].Value }
Add-Type -AssemblyName System.Drawing
if (-not ('ClarityNative' -as [type])) {
    Add-Type -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading.Tasks;
public static class ClarityNative {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] struct Point { public int X, Y; }
    public sealed class Shot {
        public string Path;
        public int Width, Height, Colors, AccentPixels;
        public double MeanLuminance;
    }
    [DllImport("user32.dll", SetLastError=true)] static extern bool SetProcessDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
    [DllImport("user32.dll", SetLastError=true)] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] static extern bool ScreenToClient(IntPtr window, ref Point point);
    [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr window);
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll")] static extern bool ShowWindowAsync(IntPtr window, int command);
    [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr window, IntPtr dc, uint flags);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern IntPtr SendMessageTimeoutW(IntPtr h, uint m, UIntPtr w, IntPtr l, uint f, uint t, out UIntPtr result);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern IntPtr SendMessageTimeoutW(IntPtr h, uint m, UIntPtr w, StringBuilder l, uint f, uint t, out UIntPtr result);
    public static void CheckOwner(IntPtr window, uint pid) {
        uint actual;
        if(window==IntPtr.Zero || GetWindowThreadProcessId(window,out actual)==0 || actual!=pid)
            throw new Exception("Refusing missing control or foreign PID");
    }
    public static void DpiAware() {
        bool process=false;
        try { process=SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch(EntryPointNotFoundException) {}
        if(!process) SetProcessDPIAware();
        // Also applies when the host already fixed its process awareness.
        EnterDpi();
    }
    static IntPtr EnterDpi() {
        try {
            var previous=SetThreadDpiAwarenessContext(new IntPtr(-4));
            if(previous==IntPtr.Zero) throw new Exception("Cannot establish per-monitor-v2 capture coordinates");
            return previous;
        } catch(EntryPointNotFoundException) { throw new Exception("Per-monitor-v2 DPI API required for reliable physical capture"); }
    }
    public static long Message(IntPtr window, uint pid, uint message, uint value, int parameter) {
        CheckOwner(window,pid); UIntPtr result;
        if(SendMessageTimeoutW(window,message,new UIntPtr(value),new IntPtr(parameter),2,2000,out result)==IntPtr.Zero)
            throw new Exception("Native message failed/timed out: "+message);
        return unchecked((long)result.ToUInt64());
    }
    public static string Text(IntPtr window, uint pid) {
        CheckOwner(window,pid); UIntPtr result; var text=new StringBuilder(262144);
        if(SendMessageTimeoutW(window,13,new UIntPtr((uint)text.Capacity),text,2,2000,out result)==IntPtr.Zero)
            throw new Exception("WM_GETTEXT failed/timed out");
        return text.ToString();
    }
    public static void ClickTab(IntPtr tab, uint pid, int index) {
        CheckOwner(tab,pid);
        uint dpi=GetDpiForWindow(tab);
        if(dpi==0) throw new Exception("Tab DPI unavailable");
        // Fixed-width native items: 108 DIP each, 34 DIP tall. No foreign-process
        // TCM_GETITEMRECT or WM_NOTIFY pointer is ever passed from this process.
        int x=(int)Math.Round((108*index+54)*dpi/96.0), y=(int)Math.Round(17*dpi/96.0);
        Rect bounds;
        if(!GetClientRect(tab,out bounds) || x>=bounds.Right || y>=bounds.Bottom)
            throw new Exception("Tab click lies outside client bounds");
        int position=(y<<16)|(x&65535);
        Message(tab,pid,0x201,1,position); // WM_LBUTTONDOWN, MK_LBUTTON
        Message(tab,pid,0x202,0,position); // Native control emits its own TCN_SELCHANGE.
    }
    public static double OriginalTopDip(IntPtr window, IntPtr parent, uint pid) {
        CheckOwner(window,pid); CheckOwner(parent,pid);
        if(GetParent(window)!=parent) return -1;
        var previous=EnterDpi();
        try {
            Rect rect; if(!GetWindowRect(window,out rect)) throw new Exception("Control rect unavailable");
            var point=new Point { X=rect.Left, Y=rect.Top };
            if(!ScreenToClient(parent,ref point)) throw new Exception("Control coordinate conversion failed");
            uint dpi=GetDpiForWindow(parent);
            if(dpi==0) throw new Exception("Parent DPI unavailable");
            return point.Y*96.0/dpi;
        } finally { SetThreadDpiAwarenessContext(previous); }
    }
    public static void Hide(IntPtr window, uint pid) {
        CheckOwner(window,pid); ShowWindowAsync(window,0);
    }
    public static Shot Capture(IntPtr window, uint pid, string path, bool clientOnly) {
        CheckOwner(window,pid);
        // PrintWindow itself has no timeout. Bound the worker and let the caller
        // shut down only its pinned child if an unresponsive target stalls it.
        var work=Task.Run(delegate {
            var previous=EnterDpi();
            try {
                CheckOwner(window,pid); Rect rect;
                bool valid=clientOnly?GetClientRect(window,out rect):GetWindowRect(window,out rect);
                if(!valid) throw new Exception("Capture rectangle unavailable");
                int width=rect.Right-rect.Left, height=rect.Bottom-rect.Top;
                if(width<1 || height<1 || width>8192 || height>8192) throw new Exception("Invalid physical capture dimensions");
                using(var bitmap=new Bitmap(width,height,PixelFormat.Format32bppArgb)) {
                    using(var graphics=Graphics.FromImage(bitmap)) {
                        graphics.Clear(Color.Magenta);
                        IntPtr dc=graphics.GetHdc();
                        try {
                            CheckOwner(window,pid);
                            if(!PrintWindow(window,dc,clientOnly?3u:2u)) throw new Exception("PrintWindow failed");
                        } finally { graphics.ReleaseHdc(dc); }
                    }
                    var colors=new HashSet<int>(); double luminance=0; int samples=0, accent=0, untouched=0;
                    for(int y=0;y<height;y+=3) for(int x=0;x<width;x+=3) {
                        Color c=bitmap.GetPixel(x,y); colors.Add(c.ToArgb()); samples++;
                        luminance+=0.2126*c.R+0.7152*c.G+0.0722*c.B;
                        if(Math.Max(c.R,Math.Max(c.G,c.B))-Math.Min(c.R,Math.Min(c.G,c.B))>45) accent++;
                        if(c.R==255 && c.G==0 && c.B==255) untouched++;
                    }
                    if(colors.Count<8 || untouched>samples/100) throw new Exception("Blank/incomplete PrintWindow capture");
                    bitmap.Save(path,ImageFormat.Png);
                    return new Shot { Path=path,Width=width,Height=height,Colors=colors.Count,
                        AccentPixels=accent,MeanLuminance=luminance/samples };
                }
            } finally { SetThreadDpiAwarenessContext(previous); }
        });
        if(!work.Wait(8000)) throw new Exception("PrintWindow exceeded eight seconds");
        return work.GetAwaiter().GetResult();
    }
}
'@
}
[ClarityNative]::DpiAware()
$session = [Diagnostics.Process]::GetCurrentProcess().SessionId
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$mutexName = "Local\AIRelay.$sid.$session.v1"
function Assert-NoInstance {
    # Opening an existing mutex never creates or takes ownership of the app mutex.
    $existing = $null
    try { $existing = [Threading.Mutex]::OpenExisting($mutexName) }
    catch [Threading.WaitHandleCannotBeOpenedException] { }
    if ($null -ne $existing) { $existing.Dispose(); throw 'Existing AI Relay instance: refusing launch or IPC' }
    $others = @(Get-Process -Name 'ai-relay' -ErrorAction SilentlyContinue | Where-Object { $_.SessionId -eq $session })
    if ($others.Count -gt 0) { throw "Existing ai-relay PID(s): $($others.Id -join ','); no automation sent" }
}
# Fail before launching anything if another main instance exists. An explicit,
# unique --config-dir also makes a launch race fail the app's directory IPC guard.
Assert-NoInstance
$outputRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\target\ui-clarity-screenshots'))
$runId = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0,8)
$output = Join-Path $outputRoot $runId
[void][IO.Directory]::CreateDirectory($output)
$temp = Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-ui-clarity-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temp)
$config = Join-Path $temp 'config.toml'
$child = $null; $mock = $null; $window = [IntPtr]::Zero
$childStart = $null; $childHandle = [IntPtr]::Zero
$failure = $null
$checks = [Collections.Generic.List[object]]::new()
$results = [Collections.Generic.List[object]]::new()
$cleanupErrors = [Collections.Generic.List[string]]::new()
$overall = [Diagnostics.Stopwatch]::StartNew()
function Record([string]$Name, [object]$Value) {
    $checks.Add([pscustomobject]@{ Check=$Name; Value=$Value; Seconds=[Math]::Round($overall.Elapsed.TotalSeconds,2) })
    Write-Output ("CHECK {0}: {1}" -f $Name, ($Value | ConvertTo-Json -Compress -Depth 5))
}
function Assert-Child {
    if ($null -eq $child -or $child.HasExited) { throw 'Owned child is missing or exited; refusing automation' }
    if ($child.Handle -ne $childHandle -or $child.StartTime -ne $childStart -or $child.Path -ine $exe) {
        throw 'Child process identity changed; refusing automation'
    }
}
function Control([int]$Id) {
    Assert-Child
    [ProxyUiSmoke]::CheckOwner($window,[uint32]$child.Id)
    $handle = [ProxyUiSmoke]::GetDlgItem($window,$Id)
    [ProxyUiSmoke]::CheckOwner($handle,[uint32]$child.Id)
    return $handle
}
function Text([int]$Id) { [ClarityNative]::Text((Control $Id),[uint32]$child.Id) }
function Wait-State([string]$Name, [scriptblock]$Test) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ($timer.Elapsed.TotalSeconds -lt 20 -and $overall.Elapsed.TotalSeconds -lt 180) {
        Assert-Child
        if (& $Test) { Record $Name $true; return }
        Start-Sleep -Milliseconds 80
    }
    throw "Bounded wait expired: $Name"
}
function Select-Page([int]$Index, [int]$VisibleControl) {
    $tab = Control 2100
    [ClarityNative]::ClickTab($tab,[uint32]$child.Id,$Index)
    Wait-State "Tab $Index selected and page control $VisibleControl visible" {
        ([ClarityNative]::Message($tab,[uint32]$child.Id,0x130B,0,0) -eq $Index) -and
        [ProxyUiSmoke]::IsWindowVisible((Control $VisibleControl))
    }
}
function Start-Gui([string]$Theme) {
    Assert-NoInstance
    $script:window = [IntPtr]::Zero
    $script:child = Start-Process -FilePath $exe -WorkingDirectory $temp -ArgumentList @('--config-dir',('"'+$temp+'"')) -PassThru
    # Hold the original process handle: never look up or kill a fresh PID by name.
    $script:childHandle = $child.Handle
    $script:childStart = $child.StartTime
    Wait-State "$Theme main window" {
        $script:window = [ProxyUiSmoke]::Find([uint32]$child.Id)
        $window -ne [IntPtr]::Zero
    }
    Assert-Child
    Record "$Theme child identity verified" @{ PID=$child.Id; Exe=$exe; StartTime=$childStart.ToString('o') }
    Wait-State "$Theme auto-start running" {
        $running = @([ProxyUiSmoke]::Children($window,'Static') | Where-Object {
            [ProxyUiSmoke]::CheckOwner($_,[uint32]$child.Id)
            [ProxyUiSmoke]::Title($_) -match '^代理状态：.*运行'
        }).Count -gt 0
        $running -and [ProxyUiSmoke]::IsWindowEnabled((Control 2003))
    }
    Select-Page 1 2332
    Wait-State "$Theme history scope initialized" { (Text 2330).Contains('数据目录已变化') }
    Record "$Theme DPI" ([ClarityNative]::GetDpiForWindow($window))
}
function Stop-Gui {
    Assert-Child
    [ProxyUiSmoke]::Post($window,[uint32]$child.Id,0x111,1002)
    if (-not $child.WaitForExit(38000)) { throw 'Owned child graceful exit exceeded 38 seconds' }
    if ($child.ExitCode -ne 0) { throw "Owned child exit code: $($child.ExitCode)" }
    Record 'owned child clean exit' $child.Id
    $child.Dispose(); $script:child=$null; $script:window=[IntPtr]::Zero
}
function Query-History([string]$Theme) {
    Wait-State "$Theme history query enabled" { [ProxyUiSmoke]::IsWindowEnabled((Control 2320)) }
    [ProxyUiSmoke]::Post((Control 2320),[uint32]$child.Id,0xF5,0)
    Wait-State "$Theme backend returned five persisted requests" {
        [ProxyUiSmoke]::IsWindowEnabled((Control 2320)) -and
        (Text 2325).Contains('匹配 5 请求') -and (Text 2325).Contains('offset 0 / 5 行')
    }
    $summary = Text 2330
    foreach ($expected in @('请求 5；完整 5 / 部分 0 / 未知 0','输入 50 / 输出 25 / 总 75','缓存命中 15','推理 10','总Token 75（覆盖 5）')) {
        if (-not $summary.Contains($expected)) { throw "History summary mismatch: $expected; actual=$summary" }
    }
    $details = @((Text 2331) -split '\r?\n' | Where-Object { $_.Contains('local-usage-model') })
    if ($details.Count -ne 5) { throw "Expected five real detail rows, got $($details.Count)" }
    foreach ($row in $details) {
        if (-not $row.Contains('10/5/15') -or -not $row.Contains('127.0.0.1') -or
            -not $row.Contains('Success') -or -not $row.Contains('Complete')) { throw "Detail mismatch: $row" }
    }
    Record "$Theme history summary" $summary
    Record "$Theme real detail rows checked" $details.Count
    Record "$Theme query status" (Text 2325)
}
function Capture-Theme([string]$Theme) {
    # This isolated history baseline expands advanced controls by default. Hide
    # ONLY visible direct children at their original top in [226,398) DIP.
    # No relayout, source edit, data change, or claim of main-workspace layout validation.
    $hidden = [Collections.Generic.List[object]]::new()

    # Children filters by class; enumerate the baseline's relevant native classes.
    foreach ($kind in @('Static','Edit','ComboBox','Button')) {
        foreach ($handle in [ProxyUiSmoke]::Children($window,$kind)) {
            Assert-Child
            [ProxyUiSmoke]::CheckOwner($handle,[uint32]$child.Id)
            if (-not [ProxyUiSmoke]::IsWindowVisible($handle) -or [ClarityNative]::GetDlgCtrlID($handle) -eq 2332) { continue }
            $top = [ClarityNative]::OriginalTopDip($handle,$window,[uint32]$child.Id)
            if ($top -ge 226 -and $top -lt 398) {
                $hidden.Add([pscustomobject]@{ Id=[ClarityNative]::GetDlgCtrlID($handle); TopDip=$top; Class=$kind })
                [ClarityNative]::Hide($handle,[uint32]$child.Id)
            }
        }
    }
    Record "$Theme isolated advanced-control adaptation" @($hidden.ToArray())
    Wait-State "$Theme advanced controls hidden, chart visible" {
        $visible = $false
        foreach ($kind in @('Static','Edit','ComboBox','Button')) {
            foreach ($handle in [ProxyUiSmoke]::Children($window,$kind)) {
                [ProxyUiSmoke]::CheckOwner($handle,[uint32]$child.Id)
                if ([ProxyUiSmoke]::IsWindowVisible($handle) -and [ClarityNative]::GetDlgCtrlID($handle) -ne 2332) {
                    $top = [ClarityNative]::OriginalTopDip($handle,$window,[uint32]$child.Id)
                    if ($top -ge 226 -and $top -lt 398) { $visible=$true }
                }
            }
        }
        -not $visible -and [ProxyUiSmoke]::IsWindowVisible((Control 2332))
    }
    $chart = Control 2332
    $rect = New-Object ClarityNative+Rect
    if (-not [ClarityNative]::GetClientRect($chart,[ref]$rect)) { throw 'Chart GetClientRect failed' }
    Assert-Child
    $chartShot = [ClarityNative]::Capture($chart,[uint32]$child.Id,(Join-Path $output "$Theme-chart.png"),$true)
    if ($chartShot.Width -ne ($rect.Right-$rect.Left) -or $chartShot.Height -ne ($rect.Bottom-$rect.Top)) {
        throw 'Physical chart capture dimensions differ from actual client rectangle'
    }
    if ($chartShot.AccentPixels -lt 20) { throw 'Chart has no meaningful colored rendering' }
    Assert-Child
    $windowShot = [ClarityNative]::Capture($window,[uint32]$child.Id,(Join-Path $output "$Theme-window-adapted.png"),$false)
    $result = [pscustomobject]@{ Theme=$Theme; PID=$child.Id; Dpi=[ClarityNative]::GetDpiForWindow($window);
        Requests=5; InputTokens=50; OutputTokens=25; TotalTokens=75; Chart=$chartShot; Window=$windowShot;
        HiddenAdvancedControls=@($hidden.ToArray()); HistoryStatus=(Text 2325) }
    $results.Add($result)
    Record "$Theme physical chart screenshot" $chartShot
    Record "$Theme adapted full-window screenshot" $windowShot
}
try {
    $mock = [ProxyUiMock]::new('clarity-local-only')
    $port = [ProxyUiSmoke]::FreePort()
    if ($port -eq $mock.Port) { throw 'Ephemeral ports collided; safely rerun' }
    foreach ($theme in @('light','dark')) {
        # Restart against the same private history: dark must restore exactly five,
        # not generate another five or reuse the former process's session usage.
        $toml = "schema_version = 1`n[upstream]`nurl = 'http://127.0.0.1:$($mock.Port)/v1'`n[listen]`naddress = '127.0.0.1'`nport = $port`n[proxy]`nauto_start = true`n[ui]`ntheme = '$theme'`n"
        [IO.File]::WriteAllText($config,$toml,[Text.UTF8Encoding]::new($false))
        Start-Gui $theme
        if ($theme -eq 'light') {
            for ($request=0; $request -lt 5; $request++) {
                Assert-Child
                [ProxyUiSmoke]::RequestBody($port,'/usage-json',[ProxyUiSmoke]::UsageJson)
            }
            Record 'five local usage-json responses verified byte-for-byte' $mock.Requests
            if ($mock.Requests -ne 5) { throw 'Mock did not receive exactly five requests' }
        }
        Query-History $theme
        Capture-Theme $theme
        Stop-Gui
    }
    if ($mock.Requests -ne 5) { throw 'Unexpected additional mock requests during restart/query/capture' }
    $light = $results[0].Chart.MeanLuminance; $dark = $results[1].Chart.MeanLuminance
    if ($light -lt 150 -or $dark -gt 120 -or ($light-$dark) -lt 60) {
        throw "Light/dark rendered chart luminance mismatch: $light / $dark (high contrast is not validated)"
    }
    Record 'distinct light/dark real chart rendering' @{ Light=$light; Dark=$dark }
} catch {
    $failure = $_
    try {
        if ($null -ne $child -and -not $child.HasExited -and $window -ne [IntPtr]::Zero) {
            Record 'failure history status' (Text 2325)
        }
    } catch { Write-Warning "Failure-state inspection unavailable: $($_.Exception.Message)" }
} finally {
    $exited = $null -eq $child
    if ($null -ne $child) {
        try {
            if (-not $child.HasExited) {
                Assert-Child
                if ($window -ne [IntPtr]::Zero) {
                    try { [ProxyUiSmoke]::Post($window,[uint32]$child.Id,0x111,1002) }
                    catch { Write-Warning "Owned-child exit notification failed: $($_.Exception.Message)" }
                }
                if (-not $child.WaitForExit(38000)) {
                    Assert-Child
                    $child.Kill()
                    if (-not $child.WaitForExit(10000)) { throw 'Owned child termination timed out' }
                }
            }
            $exited=$child.HasExited
        } catch { $cleanupErrors.Add("Owned-child cleanup failed: $($_.Exception.Message)") }
        finally { $child.Dispose(); $child=$null }
    }
    if ($null -ne $mock) {
        try { $mock.Dispose() } catch { $cleanupErrors.Add("Local mock shutdown failed: $($_.Exception.Message)") }
    }
    if ($exited) {
        for ($attempt=0; $attempt -lt 20; $attempt++) {
            try { if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }; break }
            catch {
                if ($attempt -eq 19) { $cleanupErrors.Add("Private temp retained at ${temp}: $($_.Exception.Message)") }
                else { Start-Sleep -Milliseconds 250 }
            }
        }
    } else { $cleanupErrors.Add("Child exit unconfirmed; private data retained at $temp") }
    $message = $null; if ($null -ne $failure) { $message=$failure.ToString() }
    $report = [ordered]@{
        Passed=($null -eq $failure -and $cleanupErrors.Count -eq 0); Exe=$exe; OutputDirectory=$output;
        Error=$message; CleanupErrors=@($cleanupErrors.ToArray()); TempRemoved=(-not (Test-Path -LiteralPath $temp));
        Checks=@($checks.ToArray()); Results=@($results.ToArray());
        Limits='PrintWindow rendering only; isolated child advanced controls at original top [226,398) DIP hidden for capture. Not main-workspace history layout, physical desktop, multi-monitor, high-contrast or accessibility acceptance.'
    }
    [IO.File]::WriteAllText((Join-Path $output 'state-checks.json'),($report | ConvertTo-Json -Depth 10),[Text.UTF8Encoding]::new($true))
    Write-Output "Report/screenshots: $output"
}
foreach ($cleanupError in $cleanupErrors) { Write-Warning $cleanupError }
if ($null -ne $failure) { $PSCmdlet.ThrowTerminatingError($failure) }
if ($cleanupErrors.Count -gt 0) { throw ($cleanupErrors -join '; ') }
Write-Output 'PASS: light/dark, five local requests, persisted 50/25/75 tokens, five detail rows, chart physical dimensions, owned-child exit and temporary-data cleanup.'
Write-Output 'LIMIT: isolated chart/theme PrintWindow validation with adapted advanced controls; NOT full main-workspace history layout acceptance.'
