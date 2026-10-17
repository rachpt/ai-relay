# Interactive Windows desktop. Only an owned temporary --config-dir and loopback mock.
# Refuses an existing AI Relay GUI; never edits real preferences, Startup or firewall.
[CmdletBinding()]
param([string]$ExePath, [ValidateRange(5,30)][int]$StepTimeoutSeconds = 20)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $ExePath) { $ExePath = Join-Path $PSScriptRoot '..\target\debug\ai-relay.exe' }
$exe = (Get-Item -LiteralPath $ExePath).FullName
$source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'smoke-proxy-ui.ps1'))
$match = [regex]::Match($source, "(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@")
if (-not $match.Success) { throw 'Missing PID-checked smoke helpers' }
Add-Type -TypeDefinition ($match.Groups[1].Value + @'
public static class WindowSmoke {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
    [StructLayout(LayoutKind.Sequential)] struct MonitorInfo { public uint Size; public Rect Monitor,Work; public uint Flags; }
    delegate bool EnumProc(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr data);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] static extern IntPtr MonitorFromWindow(IntPtr window, uint flags);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern bool GetMonitorInfoW(IntPtr monitor, ref MonitorInfo info);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr window,IntPtr after,int x,int y,int width,int height,uint flags);
    [DllImport("user32.dll")] static extern bool IsZoomed(IntPtr window);
    [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr window);
    [StructLayout(LayoutKind.Sequential)] struct Point { public int X,Y; }
    [StructLayout(LayoutKind.Sequential)] struct Placement { public uint Length,Flags,Show; public Point Min,Max; public Rect Normal; }
    [DllImport("user32.dll")] static extern bool GetWindowPlacement(IntPtr window, ref Placement placement);
    [DllImport("user32.dll")] static extern IntPtr GetThreadDpiAwarenessContext();
    [DllImport("user32.dll")] static extern int GetAwarenessFromDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr window,uint attribute,out Rect rect,int size);
    // PowerShell 5.1 is DPI-unaware. These scoped calls must use physical pixels
    // just like the app's per-monitor-v2 GUI; otherwise 200% doubles SetWindowPos.
    sealed class PhysicalPixels : IDisposable {
        readonly IntPtr previous;
        public PhysicalPixels() {
            previous=SetThreadDpiAwarenessContext(new IntPtr(-4));
            if(previous==IntPtr.Zero) throw new System.ComponentModel.Win32Exception("Cannot enter per-monitor-v2 DPI context");
        }
        public void Dispose() { SetThreadDpiAwarenessContext(previous); }
    }
    static string Format(Rect r) { return String.Format("[{0},{1},{2},{3}] {4}x{5}",r.Left,r.Top,r.Right,r.Bottom,r.Right-r.Left,r.Bottom-r.Top); }
    public static string Describe(IntPtr window,uint pid) {
        ProxyUiSmoke.CheckOwner(window,pid);
        int awareness=GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext());
        Rect caller;
        if(!GetWindowRect(window,out caller)) throw new Exception("GetWindowRect failed");
        using(var physicalPixels=new PhysicalPixels()) {
            Rect physical=Bounds(window,pid), frame; int dwm=DwmGetWindowAttribute(window,9,out frame,Marshal.SizeOf(typeof(Rect)));
            var p=new Placement(); p.Length=(uint)Marshal.SizeOf(p);
            if(!GetWindowPlacement(window,ref p)) throw new Exception("GetWindowPlacement failed");
            return String.Format("callerAwareness={0}; dpi={1}; visible={2}; maximized={3}; callerRect={4}; physicalRect={5}; work={6}; dwm({7})={8}; placement flags={9} show={10} normal={11}",
                awareness,Dpi(window,pid),ProxyUiSmoke.IsWindowVisible(window),Maximized(window,pid),Format(caller),Format(physical),Format(Work(window,pid)),dwm,Format(frame),p.Flags,p.Show,Format(p.Normal));
        }
    }
    public static bool Existing() {
        bool found=false;
        EnumWindows(delegate(IntPtr h,IntPtr p) { if(ProxyUiSmoke.Class(h)=="AIRelay.Window.v1") { found=true; return false; } return true; },IntPtr.Zero);
        return found;
    }
    public static Rect Bounds(IntPtr window,uint pid) {
        ProxyUiSmoke.CheckOwner(window,pid);
        using(var physicalPixels=new PhysicalPixels()) {
            Rect r; if(!GetWindowRect(window,out r)) throw new Exception("GetWindowRect failed"); return r;
        }
    }
    public static Rect Work(IntPtr window,uint pid) {
        ProxyUiSmoke.CheckOwner(window,pid);
        using(var physicalPixels=new PhysicalPixels()) {
            var m=new MonitorInfo(); m.Size=(uint)Marshal.SizeOf(m);
            if(!GetMonitorInfoW(MonitorFromWindow(window,2),ref m)) throw new Exception("GetMonitorInfo failed"); return m.Work;
        }
    }
    public static void Move(IntPtr window,uint pid,int x,int y,int width,int height) {
        ProxyUiSmoke.CheckOwner(window,pid);
        using(var physicalPixels=new PhysicalPixels()) {
            if(!SetWindowPos(window,IntPtr.Zero,x,y,width,height,0x14)) throw new Exception("SetWindowPos failed");
        }
        ProxyUiSmoke.Post(window,pid,0x232,0); // WM_EXITSIZEMOVE, not per-WM_SIZE persistence.
    }
    public static void RequestError(int port) {
        using(var client=new TcpClient()) {
            if(!client.ConnectAsync(IPAddress.Loopback,port).Wait(3000)) throw new Exception("Mock error connect timeout");
            client.ReceiveTimeout=3000; client.SendTimeout=3000;
            using(var stream=client.GetStream()) {
                byte[] request=Encoding.ASCII.GetBytes("GET /window-smoke-error HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
                stream.Write(request,0,request.Length);
                using(var reader=new StreamReader(stream)) {
                    string response=reader.ReadToEnd();
                    if(!response.StartsWith("HTTP/1.1 400") || !response.EndsWith("bad target")) throw new Exception("Expected local mock HTTP 400");
                }
            }
        }
    }
    public static bool Maximized(IntPtr window,uint pid) { ProxyUiSmoke.CheckOwner(window,pid); return IsZoomed(window); }
    public static uint Dpi(IntPtr window,uint pid) { ProxyUiSmoke.CheckOwner(window,pid); return GetDpiForWindow(window); }
}
public static class WindowTray {
    [DllImport("user32.dll",SetLastError=true)] static extern bool PostMessageW(IntPtr h,uint m,UIntPtr w,IntPtr l);
    public static void Show(IntPtr h,uint pid) { ProxyUiSmoke.CheckOwner(h,pid); if(!PostMessageW(h,0x8001,UIntPtr.Zero,new IntPtr((1<<16)|0x203))) throw new Exception("Show failed"); }
}
'@)
if ([WindowSmoke]::Existing()) { throw 'Close the existing AI Relay GUI first; refusing to automate user data' }
$temp = Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-window-ui-'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temp)
$child=$null; $mock=$null; $window=[IntPtr]::Zero; $expectedGeometry=$null
Write-Output ("EXE: {0}; lastWriteUtc={1}; SHA256={2}" -f $exe,[IO.File]::GetLastWriteTimeUtc($exe).ToString('O'),(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash)
$overall=[Diagnostics.Stopwatch]::StartNew()
function Control([int]$Id) { [ProxyUiSmoke]::GetDlgItem($window,$Id) }
function Text([int]$Id) { [ProxyUiSmoke]::ReadText((Control $Id),[uint32]$child.Id) }
function Wait([string]$Name,[scriptblock]$Test) {
    $timer=[Diagnostics.Stopwatch]::StartNew()
    while($timer.Elapsed.TotalSeconds -lt $StepTimeoutSeconds -and $overall.Elapsed.TotalSeconds -lt 180) {
        if($child.HasExited) { throw "Child exited: $Name" }
        if(& $Test) { return }
        Start-Sleep -Milliseconds 80
    }
    throw "Timeout: $Name"
}
function Click([int]$Id) {
    if(-not [ProxyUiSmoke]::IsWindowEnabled((Control $Id))) { throw "Disabled control $Id" }
    [ProxyUiSmoke]::Post((Control $Id),[uint32]$child.Id,0xF5,0)
}
function Category {
    [ProxyUiSmoke]::SelectTab($window,[uint32]$child.Id,3)
    [ProxyUiSmoke]::Send((Control 2039),[uint32]$child.Id,0x14E,5)
    [ProxyUiSmoke]::Send($window,[uint32]$child.Id,0x111,(2039 -bor (1 -shl 16)))
}
function Check([int]$Id,[bool]$Value) { [ProxyUiSmoke]::Send((Control $Id),[uint32]$child.Id,0xF1,[uint32]$Value) }
function Start-Owned([bool]$Background) {
    $arguments=@('--config-dir',('"'+$temp+'"'))
    if($Background) { $arguments += '--background' }
    $script:child=Start-Process -FilePath $exe -ArgumentList $arguments -PassThru
    Wait 'owned GUI and loaded preferences' {
        $script:window=[ProxyUiSmoke]::Find([uint32]$child.Id)
        $window -ne [IntPtr]::Zero -and [ProxyUiSmoke]::IsWindowEnabled((Control 2001))
    }
    # Allow async window response after the main config event, including hidden startup.
    Start-Sleep -Milliseconds 600
}
function Exit-Owned {
    [ProxyUiSmoke]::Post($window,[uint32]$child.Id,0x111,1002)
    if(-not $child.WaitForExit(40000) -or $child.ExitCode -ne 0) { throw 'Graceful exit failed' }
    $child.Dispose(); $script:child=$null
}
function Geometry-Matches($Expected) {
    $actual=[WindowSmoke]::Bounds($window,[uint32]$child.Id)
    return $actual.Left -eq $Expected.left -and $actual.Top -eq $Expected.top -and ($actual.Right-$actual.Left) -eq $Expected.width -and ($actual.Bottom-$actual.Top) -eq $Expected.height -and [WindowSmoke]::Dpi($window,[uint32]$child.Id) -eq $Expected.dpi -and [WindowSmoke]::Maximized($window,[uint32]$child.Id) -eq $Expected.maximized
}
function Assert-Geometry($Expected) {
    if(-not (Geometry-Matches $Expected)) {
        throw 'Actual physical window geometry does not equal the requested/saved geometry'
    }
}
function Saved-Geometry($Expected) {
    $g=[IO.File]::ReadAllText($preferences) | ConvertFrom-Json
    return $g.left -eq $Expected.left -and $g.top -eq $Expected.top -and $g.width -eq $Expected.width -and $g.height -eq $Expected.height -and $g.dpi -eq $Expected.dpi -and $g.maximized -eq $Expected.maximized
}
function Assert-Onscreen {
    $rect=[WindowSmoke]::Bounds($window,[uint32]$child.Id)
    $work=[WindowSmoke]::Work($window,[uint32]$child.Id)
    if($rect.Left -lt $work.Left -or $rect.Top -lt $work.Top -or $rect.Right -gt $work.Right -or $rect.Bottom -gt $work.Bottom) { throw 'Restored window exceeds monitor work area' }
}
try {
    $mock=[ProxyUiMock]::new('window-overview-marker')
    $port=[ProxyUiSmoke]::FreePort()
    $config=Join-Path $temp 'config.toml'
    $preferences=Join-Path $temp 'window.json'
    [IO.File]::WriteAllText($config,"schema_version = 1`n[upstream]`nurl = 'http://127.0.0.1:$($mock.Port)/v1'`n[listen]`nport = $port`n[proxy]`nauto_start = false`n[ui]`nclose_to_tray = true`nremember_window_geometry = true`n")
    $seed='{"left":-30000,"top":-30000,"width":900,"height":900,"dpi":96,"maximized":true}'
    [IO.File]::WriteAllText($preferences,$seed)
    Start-Owned $true
    if([ProxyUiSmoke]::IsWindowVisible($window)) { throw 'Background restore showed the window' }
    if([IO.File]::ReadAllText($preferences) -cne $seed) { throw 'Initial load overwrote saved geometry' }
    # Explicit show request through the owned tray callback needs both parameters.
    # WM_LBUTTONDBLCLK = 0x203; icon id=1 in lParam's high word.
    # Reuse the PID-checked helper rather than posting to any foreign HWND.
    [WindowTray]::Show($window,[uint32]$child.Id)
    Wait 'explicit show restores maximized state' { [ProxyUiSmoke]::IsWindowVisible($window) -and [WindowSmoke]::Maximized($window,[uint32]$child.Id) }
    [ProxyUiSmoke]::Post($window,[uint32]$child.Id,0x112,0xF120) # SC_RESTORE
    Wait 'restore normal window' { -not [WindowSmoke]::Maximized($window,[uint32]$child.Id) }
    Assert-Onscreen
    $work=[WindowSmoke]::Work($window,[uint32]$child.Id)
    $dpi=[WindowSmoke]::Dpi($window,[uint32]$child.Id)
    $width=[Math]::Min([int](800*$dpi/96),$work.Right-$work.Left)
    $height=[Math]::Min([int](900*$dpi/96),$work.Bottom-$work.Top)
    $left=$work.Left+[Math]::Min([int](37*$dpi/96),$work.Right-$work.Left-$width)
    $top=$work.Top+[Math]::Min([int](29*$dpi/96),$work.Bottom-$work.Top-$height)
    $expectedGeometry=[pscustomobject]@{left=$left; top=$top; width=$width; height=$height; dpi=$dpi; maximized=$false}
    Write-Output ('MOVE requested: '+($expectedGeometry | ConvertTo-Json -Compress))
    Write-Output ('BEFORE MOVE: '+[WindowSmoke]::Describe($window,[uint32]$child.Id))
    [WindowSmoke]::Move($window,[uint32]$child.Id,$left,$top,$width,$height)
    Write-Output ('AFTER MOVE: '+[WindowSmoke]::Describe($window,[uint32]$child.Id))
    Assert-Geometry $expectedGeometry
    Wait 'geometry saved in owned data directory' { Saved-Geometry $expectedGeometry }
    Write-Output ('DISK saved: '+[IO.File]::ReadAllText($preferences))
    Click 2102
    Wait 'actual listen snapshot' { (Text 2107).Contains("127.0.0.1:$port") -and [ProxyUiSmoke]::IsWindowEnabled((Control 2105)) }
    [ProxyUiSmoke]::Request($port,'window-overview-marker')
    Wait 'real overview metrics' { (Text 2107).Contains('累计请求：1') -and (Text 2107).Contains('错误：0') -and (Text 2107).Contains("127.0.0.1:$($mock.Port)") }
    [WindowSmoke]::RequestError($port)
    Wait 'real error, active and byte metrics' {
        $metrics=Text 2107
        $metrics.Contains('累计请求：2') -and $metrics.Contains('错误：1') -and $metrics.Contains('活动请求：0') -and $metrics.Contains('下载正文／隧道：32.0 B') -and $metrics -match '程序会话时长：0 天 00:00:(?!00)\d{2}'
    }
    Write-Output ('METRICS: '+(Text 2107))
    # Keep a draft listen port different from the saved one: restart must not use it.
    [ProxyUiSmoke]::SelectTab($window,[uint32]$child.Id,3)
    $edits=@([ProxyUiSmoke]::Children($window,'Edit') | Where-Object { [ProxyUiSmoke]::IsWindowVisible($_) })
    if($edits.Count -lt 3) { throw 'Settings edit controls unavailable' }
    [ProxyUiSmoke]::SetText($edits[2],[uint32]$child.Id,'1')
    [ProxyUiSmoke]::SelectTab($window,[uint32]$child.Id,0)
    Click 2104
    Wait 'restart finishes using saved listener' {
        $completed=$false
        foreach($label in [ProxyUiSmoke]::Children($window,'Static')) {
            if(([ProxyUiSmoke]::Title($label)).Contains('代理运行中；实际监听')) { $completed=$true }
        }
        $completed -and [ProxyUiSmoke]::IsWindowEnabled((Control 2104)) -and [ProxyUiSmoke]::IsWindowEnabled((Control 2105))
    }
    [ProxyUiSmoke]::Request($port,'window-overview-marker')
    Wait 'counters survive restart' { (Text 2107).Contains('累计请求：3') -and (Text 2107).Contains('错误：1') }
    [ProxyUiSmoke]::Post($window,[uint32]$child.Id,0x10,0)
    Wait 'first close-to-tray notice' { [ProxyUiSmoke]::Dialog([uint32]$child.Id) -ne [IntPtr]::Zero }
    $dialog=[ProxyUiSmoke]::Dialog([uint32]$child.Id)
    [ProxyUiSmoke]::Post($dialog,[uint32]$child.Id,0x10,0)
    Wait 'window hidden, proxy remains running' { -not [ProxyUiSmoke]::IsWindowVisible($window) }
    [ProxyUiSmoke]::Request($port,'window-overview-marker')
    $hiddenText=Text 2107
    Start-Sleep -Milliseconds 1300
    if((Text 2107) -cne $hiddenText) { throw 'Hidden overview controls were rewritten' }
    [WindowTray]::Show($window,[uint32]$child.Id)
    Wait 'show updates pending metrics' { (Text 2107).Contains('累计请求：4') -and (Text 2107).Contains('错误：1') }
    # Discard the deliberate draft port before normally saving UI preferences.
    [ProxyUiSmoke]::SelectTab($window,[uint32]$child.Id,3)
    [ProxyUiSmoke]::Send((Control 2039),[uint32]$child.Id,0x14E,4)
    [ProxyUiSmoke]::Send($window,[uint32]$child.Id,0x111,(2039 -bor (1 -shl 16)))
    Click 2037
    Category
    Check 2063 $false
    Click 2001
    Wait 'close policy saved' { [IO.File]::ReadAllText($config).Contains('close_to_tray = false') -and [ProxyUiSmoke]::IsWindowEnabled((Control 2001)) }
    [ProxyUiSmoke]::Post($window,[uint32]$child.Id,0x10,0)
    if(-not $child.WaitForExit(40000) -or $child.ExitCode -ne 0) { throw 'WM_CLOSE with close_to_tray=false did not gracefully exit' }
    $child.Dispose(); $child=$null
    if(-not [ProxyUiSmoke]::PortClosed($port)) { throw 'Listener remained after exit' }
    if(-not (Saved-Geometry $expectedGeometry)) { throw 'Graceful exit changed saved normal geometry' }
    Start-Owned $true
    if([ProxyUiSmoke]::IsWindowVisible($window)) { throw 'Saved normal geometry restore showed a background window' }
    Wait 'saved geometry restored asynchronously after config load' { Geometry-Matches $expectedGeometry }
    if(-not (Saved-Geometry $expectedGeometry)) { throw 'Reload overwrote persisted geometry' }
    Write-Output ('ROUNDTRIP: '+[WindowSmoke]::Describe($window,[uint32]$child.Id))
    Exit-Owned
    # Disabled geometry memory must neither restore nor rewrite an existing file.
    $saved=[IO.File]::ReadAllText($config).Replace('remember_window_geometry = true','remember_window_geometry = false')
    [IO.File]::WriteAllText($config,$saved)
    [IO.File]::WriteAllText($preferences,$seed)
    Start-Owned $false
    Assert-Onscreen
    Exit-Owned
    if([IO.File]::ReadAllText($preferences) -cne $seed) { throw 'Disabled window memory rewrote saved geometry' }
    Write-Output 'PASS: background/no-default-overwrite, removed-monitor clamp, maximized restore, exact physical-pixel async geometry save/restart roundtrip, real success/error/bytes/uptime overview metrics, saved-config restart, hidden refresh suppression, close-to-tray and graceful close-off, disabled geometry memory.'
    Write-Output 'LIMITS: no real user data/keys/Startup/firewall; clipboard is not modified by this test. Physical multimonitor/DPI changes and session shutdown remain manual acceptance.'
} catch {
    Write-Output ('FAIL: '+$_.Exception.Message)
    if($null -ne $expectedGeometry) { Write-Output ('EXPECTED geometry: '+($expectedGeometry | ConvertTo-Json -Compress)) }
    if($null -ne $child) {
        Write-Output ("Owned child PID={0} exited={1}" -f $child.Id,$child.HasExited)
        if(-not $child.HasExited -and $window -ne [IntPtr]::Zero) {
            try {
                Write-Output ([WindowSmoke]::Describe($window,[uint32]$child.Id))
                foreach($label in [ProxyUiSmoke]::Children($window,'Static')) {
                    $labelText=[ProxyUiSmoke]::Title($label)
                    if($labelText.StartsWith('代理状态：') -or $labelText.StartsWith('实际数据目录：') -or $labelText.Contains('窗口偏好')) { Write-Output ('STATE: '+$labelText) }
                }
                Write-Output ('OVERVIEW: '+(Text 2107))
            } catch { Write-Output ('Window diagnostics unavailable: '+$_.Exception.Message) }
        }
    }
    $diagnosticPath=Join-Path $temp 'window.json'
    if([IO.File]::Exists($diagnosticPath)) {
        Write-Output ('DISK window.json: '+[IO.File]::ReadAllText($diagnosticPath))
        Write-Output ('DISK lastWriteUtc: '+[IO.File]::GetLastWriteTimeUtc($diagnosticPath).ToString('O'))
    }
    Write-Output $_.ScriptStackTrace
    throw
} finally {
    # Only the Process returned by our Start-Process is terminated, never a global
    # AI Relay process lookup or CLI quit. Directory removal follows child shutdown.
    $childStopped=$null -eq $child
    try {
        if($null -ne $child) {
            try {
                if(-not $child.HasExited) {
                    $child.Kill()
                    if(-not $child.WaitForExit(10000)) { throw 'Owned test child did not terminate; retaining temporary data' }
                }
                $childStopped=$child.HasExited
            } finally { $child.Dispose() }
        }
    } finally {
        try { if($null -ne $mock) { $mock.Dispose() } }
        finally {
            if($childStopped -and [IO.Directory]::Exists($temp)) { Remove-Item -LiteralPath $temp -Recurse -Force }
        }
    }
}
