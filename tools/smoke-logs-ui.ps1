# Only test-created GUI/temp directory and loopback mock; no real credentials.
[CmdletBinding()]
param([string]$ExePath, [ValidateRange(5,30)][int]$StepTimeoutSeconds = 15, [switch]$RequireAccessLog)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
function Invoke-LogsSmoke {
    function Read-Shared([string]$Path) {
        $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
        $reader = [IO.StreamReader]::new($stream)
        try { $reader.ReadToEnd() } finally { $reader.Dispose() }
    }
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class LogsUiRegression {
    delegate bool EnumProc(IntPtr hwnd, IntPtr parameter);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc callback, IntPtr parameter);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll", EntryPoint="GetDlgItem")] static extern IntPtr NativeGetDlgItem(IntPtr hwnd, int id);
    [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr hwnd);
    [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr hwnd);
    static IntPtr GetDlgItem(IntPtr parent, int id) {
        var found=NativeGetDlgItem(parent,id);
        if(found!=IntPtr.Zero) return found;
        EnumChildWindows(parent,delegate(IntPtr h, IntPtr p) { if(GetDlgCtrlID(h)==id) { found=h; return false; } return true; },IntPtr.Zero);
        return found;
    }
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int length);
    static string Class(IntPtr hwnd) { var b=new StringBuilder(256); GetClassNameW(hwnd,b,b.Capacity); return b.ToString(); }
    static void CheckOwner(IntPtr hwnd, uint pid) {
        uint owner; GetWindowThreadProcessId(hwnd,out owner);
        if(hwnd==IntPtr.Zero || owner!=pid) throw new Exception("Refusing another process or missing control");
    }
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int w, int h, uint flags);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern IntPtr SendMessageTimeoutW(IntPtr hwnd, uint msg, UIntPtr wp, StringBuilder lp, uint flags, uint timeout, out UIntPtr result);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern IntPtr SendMessageTimeoutW(IntPtr hwnd, uint msg, UIntPtr wp, string lp, uint flags, uint timeout, out UIntPtr result);
    public static bool SetTextRaw(IntPtr hwnd, uint pid, string text) {
        CheckOwner(hwnd,pid); UIntPtr result;
        if(SendMessageTimeoutW(hwnd,12,UIntPtr.Zero,text,2,1000,out result)==IntPtr.Zero) throw new Exception("WM_SETTEXT timed out/failed");
        return result!=UIntPtr.Zero;
    }
    public static string Read(IntPtr hwnd, uint pid) {
        CheckOwner(hwnd,pid);
        var buffer = new StringBuilder(262145); UIntPtr result;
        if(SendMessageTimeoutW(hwnd,13,new UIntPtr((uint)buffer.Capacity),buffer,2,1000,out result)==IntPtr.Zero) throw new Exception("Log text read timed out");
        return buffer.ToString();
    }
    public static Rect Bounds(IntPtr hwnd) {
        Rect rect; if(!GetWindowRect(hwnd,out rect)) throw new Exception("Missing window bounds"); return rect;
    }
    public static void Resize(IntPtr hwnd, uint pid, int width, int height) {
        CheckOwner(hwnd,pid);
        if(!SetWindowPos(hwnd,IntPtr.Zero,0,0,width,height,0x16)) throw new Exception("Test child resize failed");
    }
    public static void CheckLayout(IntPtr window, uint pid) {
        CheckOwner(window,pid); Rect client;
        if(!GetClientRect(window,out client)) throw new Exception("Missing client bounds");
        var host=GetDlgItem(window,2400);
        CheckOwner(host,pid);
        var screen=Bounds(host); // Page children may extend vertically beyond the scroll viewport.
        uint dpi=GetDpiForWindow(window);
        var all=new System.Collections.Generic.List<IntPtr>();
        EnumChildWindows(host, delegate(IntPtr h, IntPtr parameter) {
            CheckOwner(h,pid);
            string kind=Class(h);
            if(GetParent(h)==host && IsWindowVisible(h) && (kind.Equals("Edit",StringComparison.OrdinalIgnoreCase) || kind.Equals("Button",StringComparison.OrdinalIgnoreCase) || kind.Equals("Static",StringComparison.OrdinalIgnoreCase) || kind.Equals("ComboBox",StringComparison.OrdinalIgnoreCase) || kind.Equals("SysDateTimePick32",StringComparison.OrdinalIgnoreCase))) all.Add(h);
            return true;
        },IntPtr.Zero);
        // Compare virtual page siblings, not the viewport container or fixed footer.
        for(int id=2200;id<=2229;id++) {
            var h=GetDlgItem(window,id);
            if(h==IntPtr.Zero || !IsWindowVisible(h)) continue;
            if(GetParent(h)!=host) throw new Exception("Log control outside page viewport "+id);
            var r=Bounds(h);
            if(r.Right-r.Left<1 || r.Bottom-r.Top<1) throw new Exception("Empty log control "+id);
            if(r.Right>screen.Right || r.Left<screen.Left) throw new Exception("Clipped log control "+id);
            foreach(var other in all) {
                if(h==other) continue;
                var b=Bounds(other);
                // Drop-down list extents are not occupied while the combo is closed.
                if(Class(h).Equals("ComboBox",StringComparison.OrdinalIgnoreCase)) r.Bottom=Math.Min(r.Bottom,r.Top+(int)(30*dpi/96));
                if(Class(other).Equals("ComboBox",StringComparison.OrdinalIgnoreCase)) b.Bottom=Math.Min(b.Bottom,b.Top+(int)(30*dpi/96));
                if(r.Left<b.Right && r.Right>b.Left && r.Top<b.Bottom && r.Bottom>b.Top)
                    throw new Exception("Overlapping log control "+id+" with "+Class(other));
            }
        }
    }
}
'@
    function LogText { [LogsUiRegression]::Read((Button 2200), [uint32]$child.Id) }
    function Set-LogFilter([int]$Id, [string]$Value) { [ProxyUiSmoke]::SetText((Button $Id), [uint32]$child.Id, $Value) }
    function Select-LogFilter([int]$Id, [int]$Index) { [ProxyUiSmoke]::Send((Button $Id), [uint32]$child.Id, 0x14E, [uint32]$Index) }
    function Reset-LogFilters {
        foreach ($id in @(2207,2224,2225,2227)) { Set-LogFilter $id '' }
        Select-LogFilter 2208 0
        Select-LogFilter 2226 0
        [ProxyUiSmoke]::Send((Button 2228), [uint32]$child.Id, 0xF1, 0)
        Wait-Until 'filter validation cleared' { -not (LogStatus).Contains('筛选无效') }
    }
    function Assert-Sequences([int[]]$Expected) {
        $text = LogText
        $actual = @([regex]::Matches($text, '"sequence":(\d+)') | ForEach-Object { [int]$_.Groups[1].Value })
        if (($actual -join ',') -ne ($Expected -join ',')) { throw "Unexpected filtered sequences: $($actual -join ','); expected $($Expected -join ',')" }
    }
    function LogStatus { [ProxyUiSmoke]::ReadText((Button 2220), [uint32]$child.Id) }
    function LogClick([int]$Id) { Select-Page 2; Click $Id }
    function Apply-Options {
        LogClick 2219
        Wait-Until 'log options applied without persistence' { (LogStatus).Contains('未持久化') -and [ProxyUiSmoke]::IsWindowEnabled((Button 2219)) }
    }
    function Answer([uint32]$Id) {
        $script:dialog = [IntPtr]::Zero
        Wait-Until 'test-owned deletion confirmation' {
            $script:dialog = [ProxyUiSmoke]::Dialog([uint32]$child.Id)
            $script:dialog -ne [IntPtr]::Zero
        }
        # Only a dialog belonging to this exact test child; IDYES=6, IDNO=7.
        $target = [ProxyUiSmoke]::GetDlgItem($script:dialog, [int]$Id)
        $answered = $script:dialog
        [ProxyUiSmoke]::Post($target, [uint32]$child.Id, 0xF5, 0)
        Wait-Until 'answered confirmation closed' { [ProxyUiSmoke]::Dialog([uint32]$child.Id) -ne $answered }
    }
    Select-Page 3
    $network = @([ProxyUiSmoke]::Children($window, 'Edit') | Where-Object { [ProxyUiSmoke]::IsWindowVisible($_) })
    [ProxyUiSmoke]::SetText($network[0], [uint32]$child.Id, 'http://127.0.0.1:1/unsaved')
    Select-Page 2
    Wait-Until 'real runtime log snapshot' { (LogText).Contains('Runtime') }
    $originalBounds = [LogsUiRegression]::Bounds($window)
    $dpi = [LogsUiRegression]::GetDpiForWindow($window)
    try {
        foreach ($size in @(@(760,880), @(900,1000))) {
            [LogsUiRegression]::Resize($window, [uint32]$child.Id, [int]($size[0]*$dpi/96), [int]($size[1]*$dpi/96))
            Start-Sleep -Milliseconds 200
            [LogsUiRegression]::CheckLayout($window, [uint32]$child.Id)
        }
    } finally {
        [LogsUiRegression]::Resize($window, [uint32]$child.Id, ($originalBounds.Right-$originalBounds.Left), ($originalBounds.Bottom-$originalBounds.Top))
    }
    Write-Output "PASS: logs layout at actual DPI $dpi, minimum and enlarged child window sizes."
    LogClick 2202
    Wait-Until 'live runtime view paused for metadata filters' { (LogStatus).Contains('已暂停') }
    $runtimeLine = @((LogText) -split "`n" | Where-Object { $_ -match '^\d+ #\d+ Runtime ' })[0].Trim()
    $runtimeEpoch = [long]([regex]::Match($runtimeLine, '^(\d+) ').Groups[1].Value)
    $runtimeDay = [DateTimeOffset]::FromUnixTimeSeconds($runtimeEpoch).UtcDateTime.ToString('yyyy-MM-dd')
    Select-LogFilter 2226 1
    Set-LogFilter 2224 $runtimeDay
    Set-LogFilter 2225 $runtimeDay
    Set-LogFilter 2207 'runtime'
    Wait-Until 'live UTC date and Runtime metadata AND search' { (LogText).Contains($runtimeLine) -and -not (LogStatus).Contains('筛选无效') }
    if ((LogText).Contains('Access')) { throw 'Runtime category included Access' }
    Set-LogFilter 2227 '2xx'
    Wait-Until 'runtime has no HTTP status' { (LogText).Length -eq 0 -and -not (LogStatus).Contains('筛选无效') }
    Reset-LogFilters
    LogClick 2202
    [ProxyUiSmoke]::SetText((Button 2216), [uint32]$child.Id, '4')
    Apply-Options
    if ([IO.File]::ReadAllText($configPath) -cne $saved) { throw 'Immediate log application rewrote config' }
    Select-Page 3
    if ([ProxyUiSmoke]::ReadText($network[0], [uint32]$child.Id) -ne 'http://127.0.0.1:1/unsaved') { throw 'Log application overwrote network draft' }
    [ProxyUiSmoke]::SetText($network[0], [uint32]$child.Id, "http://127.0.0.1:$($mock.Port)/v1")
    Click 2001
    Wait-Until 'normal save persists logging model' { [ProxyUiSmoke]::IsWindowEnabled((Button 2001)) -and [IO.File]::ReadAllText($configPath).Contains('retained_files = 4') }
    Wait-Until 'local proxy running for log capture' { (Status).StartsWith('代理状态：运行中') }
    [ProxyUiSmoke]::Request($port, $marker)
    Select-Page 2
    # Advanced live filters require actual typed Access entries in every run;
    # -RequireAccessLog remains accepted for callers of the earlier smoke.
    Wait-Until 'real typed proxy access metadata appears' { (LogText).Contains('Access') -and (LogText).Contains('status: Some(200)') }
    LogClick 2202
    Wait-Until 'live access batch frozen' { (LogStatus).Contains('已暂停') }
    $accessLine = @((LogText) -split "`n" | Where-Object { $_ -match '^\d+ #\d+ Access .*status: Some\(200\)' })[0].Trim()
    $accessEpoch = [long]([regex]::Match($accessLine, '^(\d+) ').Groups[1].Value)
    $accessDay = [DateTimeOffset]::FromUnixTimeSeconds($accessEpoch).UtcDateTime.ToString('yyyy-MM-dd')
    Set-LogFilter 2224 $accessDay
    Set-LogFilter 2225 $accessDay
    Select-LogFilter 2226 2
    Select-LogFilter 2208 3
    Set-LogFilter 2227 '200'
    Set-LogFilter 2207 'access'
    Wait-Until 'live Access/UTC-same-day/exact-status/level/text AND' { (LogText).Contains($accessLine) -and -not (LogText).Contains('Runtime') -and -not (LogStatus).Contains('筛选无效') }
    Set-LogFilter 2227 '2xx'
    Wait-Until 'live status class' { (LogText).Contains($accessLine) }
    $frozenLive = LogText
    $failure = [Net.HttpWebRequest]::Create("http://127.0.0.1:$port/logs-filter-failure")
    $failure.Proxy = $null
    $failure.Timeout = 2000
    $failure.ReadWriteTimeout = 2000
    try {
        $response = $failure.GetResponse()
        $response.Close()
        throw 'Expected loopback mock HTTP 400'
    } catch [Net.WebException] {
        if ($null -eq $_.Exception.Response) { throw }
        $response = $_.Exception.Response
        try { if ([int]$response.StatusCode -ne 400) { throw "Unexpected mock failure status: $($response.StatusCode)" } }
        finally { $response.Close() }
    }
    Apply-Options
    Start-Sleep -Milliseconds 1200
    if ((LogText) -cne $frozenLive) { throw 'Paused live metadata/text changed with newer backend snapshot' }
    Set-LogFilter 2227 '4xx'
    Wait-Until 'paused metadata excludes newly recorded failure' { (LogText).Length -eq 0 }
    Set-LogFilter 2227 '200'
    Wait-Until 'paused filters still use frozen success entry' { (LogText).Contains($accessLine) }
    $validLive = LogText
    Set-LogFilter 2224 ([DateTimeOffset]::FromUnixTimeSeconds($accessEpoch).UtcDateTime.AddDays(1).ToString('yyyy-MM-dd'))
    Wait-Until 'live inverted range visible' { (LogStatus).Contains('不得晚于') }
    if ((LogText) -cne $validLive) { throw 'Invalid live range changed display' }
    Reset-LogFilters
    Select-LogFilter 2226 2
    Set-LogFilter 2227 '400-499'
    LogClick 2228
    LogClick 2202
    Wait-Until 'resume uses latest typed failure and errors-only' { (LogText).Contains('status: Some(400)') -and -not (LogText).Contains('status: Some(200)') -and -not (LogText).Contains('Runtime') }
    Set-LogFilter 2207 'ConfigurationApplied'
    Wait-Until 'live text remains AND with error metadata' { (LogText).Length -eq 0 }
    Reset-LogFilters
    Wait-Until 'live unfiltered debug display restored' { (LogText).Contains('Runtime') -and (LogText).Contains('Access') }
    LogClick 2204
    Wait-Until 'clear view cursor applied' { (LogText).Length -eq 0 -and (LogStatus).Contains('已清空视图') }
    $current = Join-Path $temp 'logs\ai-relay.current.log'
    if (-not (Test-Path -LiteralPath $current)) { throw 'Clear view deleted current file' }
    LogClick 2202
    Wait-Until 'view paused' { (LogStatus).Contains('已暂停') }
    [ProxyUiSmoke]::Request($port, $marker)
    Apply-Options
    Start-Sleep -Milliseconds 1200
    if ((LogText).Length -ne 0) { throw 'Paused view changed' }
    if (-not (Read-Shared $current).Contains('configuration_applied')) { throw 'Pause stopped file recording' }
    LogClick 2202
    Wait-Until 'resume receives post-clear log entries' { (LogText).Contains('ConfigurationApplied') }
    [ProxyUiSmoke]::SetText((Button 2207), [uint32]$child.Id, 'no-match-smoke-filter')
    Wait-Until 'text filter applied' { (LogText).Length -eq 0 }
    [ProxyUiSmoke]::SetText((Button 2207), [uint32]$child.Id, '')
    [ProxyUiSmoke]::Send((Button 2208), [uint32]$child.Id, 0x14E, 5)
    Wait-Until 'error level excludes info logs' { (LogText).Length -eq 0 }
    [ProxyUiSmoke]::Send((Button 2208), [uint32]$child.Id, 0x14E, 0)
    Wait-Until 'reset filters restores real logs' { (LogText).Contains('Runtime') }
    $history = Join-Path $temp 'logs\ai-relay.1.log'
    [IO.File]::WriteAllText($history, '{"sequence":1,"event":{"level":"info","category":"runtime","event":"started"}}' + "`n")
    $other = Join-Path $temp 'logs\unowned-smoke.txt'
    [IO.File]::WriteAllText($other, 'preserve')
    LogClick 2205
    Wait-Until 'file listing completed' { (LogStatus).Contains('已刷新') -and [ProxyUiSmoke]::IsWindowEnabled((Button 2206)) }
    LogClick 2206
    Wait-Until 'real readonly file tail' { (LogText).Contains('"sequence"') -and (LogStatus).Contains('每秒自动跟随') }
    [ProxyUiSmoke]::Send((Button 2209), [uint32]$child.Id, 0x14E, 1)
    LogClick 2206
    Wait-Until 'selected history tail read' { (LogText).Contains('"started"') -and (LogStatus).Contains('ai-relay.1.log') }
    # Replace only this test-owned, closed history file with schema-shaped JSONL.
    $fixture = @(
        '{"sequence":1,"timestamp":1704067200,"event":{"category":"runtime","level":"info","event":"started","error":null},"text":"status 503 error timeout"}',
        '{"sequence":2,"timestamp":1704067201,"event":{"category":"access","level":"info","metadata":{"status":200,"error":null}},"text":"503 error"}',
        '{"sequence":3,"timestamp":1704067202,"event":{"category":"access","level":"warn","metadata":{"status":503,"error":null}}}',
        '{"sequence":4,"timestamp":1704067203,"event":{"category":"access","level":"info","metadata":{"status":null,"error":"timeout"}}}',
        '{"sequence":5,"timestamp":1704153599,"event":{"category":"runtime","level":"error","event":"operation_failed","error":"io"}}',
        '{"sequence":6,"timestamp":1704153600,"event":{"category":"access","level":"info","metadata":{"status":200,"error":null}}}'
    ) -join "`n"
    [IO.File]::WriteAllText($history, $fixture + "`n")
    LogClick 2206
    Wait-Until 'structured fixture loaded' { (LogText).Contains('"sequence":6') }
    LogClick 2202
    Wait-Until 'fixture follow paused' { (LogStatus).Contains('已暂停') }
    $fixtureBytes = [IO.File]::ReadAllBytes($history)
    Set-LogFilter 2224 '2024-01-01'
    Set-LogFilter 2225 '2024-01-01'
    Wait-Until 'UTC whole-day inclusive bounds' { (LogText).Contains('"sequence":5') -and -not (LogText).Contains('"sequence":6') }
    Assert-Sequences @(1,2,3,4,5)
    Select-LogFilter 2226 2
    Set-LogFilter 2227 '5xx'
    Set-LogFilter 2207 'metadata'
    Select-LogFilter 2208 4
    LogClick 2228
    Wait-Until 'all metadata conditions AND while paused' { (LogText).Contains('"sequence":3') -and @((LogText) -split "`n" | Where-Object { $_.Trim() }).Count -eq 1 }
    Assert-Sequences @(3)
    Set-LogFilter 2224 '2024-01-01'
    Set-LogFilter 2225 '2024-01-01'
    Wait-Until 'UTC same-day range retains metadata AND result' { (LogText).Contains('"sequence":3') -and -not (LogStatus).Contains('筛选无效') }
    Assert-Sequences @(3)
    Set-LogFilter 2207 'started'
    Wait-Until 'text is AND not OR' { (LogText).Length -eq 0 }
    Set-LogFilter 2207 'metadata'
    Select-LogFilter 2208 3
    Start-Sleep -Milliseconds 400
    Assert-Sequences @()
    Reset-LogFilters
    Set-LogFilter 2227 '200'
    Wait-Until 'exact status excludes incidental strings' { (LogText).Contains('"sequence":6') -and -not (LogText).Contains('"sequence":1') }
    Assert-Sequences @(2,6)
    Set-LogFilter 2227 '400-599'
    Wait-Until 'status interval' { (LogText).Contains('"sequence":3') }
    Assert-Sequences @(3)
    $validDisplay = LogText
    foreach ($invalid in @('6xx','500-400','status:200')) {
        Set-LogFilter 2227 $invalid
        Wait-Until 'invalid status is visible' { (LogStatus).Contains('筛选无效') }
        if ((LogText) -cne $validDisplay) { throw 'Invalid status silently changed display' }
    }
    Set-LogFilter 2227 '400-599'
    foreach ($id in @(2224,2225)) {
        Set-LogFilter $id '2024-01-01'
        Wait-Until 'valid UTC date filter applied before rejection checks' { (LogText).Contains('"sequence":3') -and -not (LogStatus).Contains('筛选无效') }
        Assert-Sequences @(3)
        $validDate = [ProxyUiSmoke]::ReadText((Button $id), [uint32]$child.Id)
        $validDisplay = LogText
        # Epoch remains covered by pure utc_bound tests; GUI pickers accept only ISO/empty.
        foreach ($invalid in @('2023-02-29','2024-13-01','2024-1-1','1704067202')) {
            if ([LogsUiRegression]::SetTextRaw((Button $id), [uint32]$child.Id, $invalid)) { throw "Date picker $id accepted invalid text: $invalid" }
            if ([ProxyUiSmoke]::ReadText((Button $id), [uint32]$child.Id) -cne $validDate) { throw "Rejected date changed picker $id value" }
            if ((LogText) -cne $validDisplay) { throw 'Rejected date changed display' }
        }
    }
    Set-LogFilter 2224 '2024-01-02'
    Set-LogFilter 2225 '2024-01-01'
    Wait-Until 'inverted UTC range is visible' { (LogStatus).Contains('不得晚于') }
    Reset-LogFilters
    LogClick 2228
    Wait-Until 'errors include HTTP failure, structured code, error level' { (LogText).Contains('"sequence":5') -and -not (LogText).Contains('"sequence":2') }
    Assert-Sequences @(3,4,5)
    Select-Page 0
    Select-Page 2
    Assert-Sequences @(3,4,5)
    if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($history)) -ne [Convert]::ToBase64String($fixtureBytes)) { throw 'Pause/filter/tab switch modified file' }
    Reset-LogFilters
    LogClick 2204
    Wait-Until 'clear file display only' { (LogText).Length -eq 0 }
    if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($history)) -ne [Convert]::ToBase64String($fixtureBytes)) { throw 'Clear view modified file' }
    # Clear view retains follow cursor: append while paused, then resume without tail reload.
    [IO.File]::AppendAllText($history, '{"sequence":7,"timestamp":1704153601,"event":{"category":"runtime","level":"info","event":"follow-smoke","error":null}}' + "`n")
    Start-Sleep -Milliseconds 1100
    if ((LogText).Length -ne 0) { throw 'Paused file view followed append' }
    LogClick 2202
    Wait-Until 'resumed follow uses retained cursor' { (LogText).Contains('"sequence":7') }
    Assert-Sequences @(7)
    # Only manipulate our own closed history file, never the live writer's file.
    [IO.File]::AppendAllText($history, '{"sequence":2,"event":{"level":"info","event":"follow-smoke"}}' + "`n")
    Wait-Until 'automatic file append follow' { (LogText).Contains('follow-smoke') }
    [IO.File]::WriteAllText($history, '{"sequence":3,"event":{"level":"info","event":"short"}}' + "`n")
    Wait-Until 'truncation clears identity cursor' { (LogStatus).Contains('变化') -and (LogStatus).Contains('停止跟随') }
    LogClick 2206
    Wait-Until 'explicit tail restarts after truncation' { (LogText).Contains('short') }
    $lines = [Collections.Generic.List[string]]::new()
    for ($i=0; $i -lt 4000; $i++) { $lines.Add('{"sequence":' + $i + ',"event":{"level":"info","event":"page-' + $i + '"}}') }
    [IO.File]::WriteAllText($history, [string]::Join("`n", $lines) + "`n")
    LogClick 2206
    Wait-Until 'typed before cursor available' { [ProxyUiSmoke]::IsWindowEnabled((Button 2222)) -and (LogText).Contains('page-3999') }
    if (@((LogText) -split "`n" | Where-Object { $_.Trim() }).Count -gt 1000) { throw 'File view exceeds 1000 lines' }
    $pagedBytes = [IO.File]::ReadAllBytes($history)
    Set-LogFilter 2227 '2xx'
    Wait-Until 'missing metadata does not match status' { (LogText).Length -eq 0 }
    Reset-LogFilters
    LogClick 2222
    Wait-Until 'previous page and paused follow' { (LogStatus).Contains('跟随已暂停') -and [ProxyUiSmoke]::IsWindowEnabled((Button 2206)) }
    if (@((LogText) -split "`n" | Where-Object { $_.Trim() }).Count -gt 1000) { throw 'Previous page exceeds 1000 lines' }
    if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($history)) -ne [Convert]::ToBase64String($pagedBytes)) { throw 'Paging modified file' }
    [ProxyUiSmoke]::Send((Button 2209), [uint32]$child.Id, 0x14E, 0)
    LogClick 2206
    Wait-Until 'selected current tail read' { (LogStatus).Contains('ai-relay.current.log') -and (LogText).Contains('"sequence"') }
    LogClick 2210
    Answer 7
    Wait-Until 'cancelled clear preview' { (LogStatus).Contains('已取消') }
    if (-not (Test-Path -LiteralPath $history)) { throw 'Cancel deleted history' }
    # Deny delete sharing to force a real partial cleanup report.
    $locked = [IO.File]::Open($history, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        LogClick 2210
        Answer 6
        Wait-Until 'partial clear failure reported' { (LogStatus).Contains('失败 1 项') }
    } finally { $locked.Dispose() }
    LogClick 2210
    Answer 6
    Wait-Until 'history deleted after confirmation' { (LogStatus).Contains('失败 0 项') -and -not (Test-Path -LiteralPath $history) }
    if (-not (Test-Path -LiteralPath $current)) { throw 'History cleanup deleted current' }
    LogClick 2211
    Answer 6
    Answer 7
    Wait-Until 'all clear second confirmation cancelled' { (LogStatus).Contains('已取消') }
    LogClick 2211
    Answer 6
    Answer 6
    Wait-Until 'all application log cleanup completed' { (LogStatus).Contains('全部应用日志清理') -and (LogStatus).Contains('失败 0 项') }
    if ([IO.File]::ReadAllText($other) -ne 'preserve' -or -not (Test-Path -LiteralPath $configPath)) { throw 'Cleanup touched unowned files/config' }
    LogClick 2201
    Apply-Options
    Wait-Until 'logging continues after clear all' { (LogText).Contains('ConfigurationApplied') }
    if ((Read-Shared $current).Contains('probe=1')) { throw 'Query values leaked into managed logs' }
    Write-Output 'PASS: typed live snapshots with original Debug display, live/file UTC/date/category/status/errors/text/level AND, invalid-filter preservation, paused filters, cursor/follow/page, 1000-line bound, DPI-aware layout, non-destructive view operations, apply/save and cleanup regression.'
    Write-Output 'LIMITS: synthetic messages; no physical keyboard or multi-monitor DPI validation; no real Startup writes.'
}
& (Join-Path $PSScriptRoot 'smoke-proxy-ui.ps1') -ExePath $ExePath -StepTimeoutSeconds $StepTimeoutSeconds -LogsOnly
