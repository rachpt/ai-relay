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
    function LogText { [ProxyUiSmoke]::ReadText((Button 2200), [uint32]$child.Id) }
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
    if ($RequireAccessLog) {
        Wait-Until 'real proxy access metadata appears' { (LogText).Contains('Access') }
    } else {
        Write-Output 'LIMIT: request forwarded, but access observer bridge is not asserted; use -RequireAccessLog after controller start_with_observers integration.'
    }
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
    Wait-Until 'typed before cursor available' { [ProxyUiSmoke]::IsWindowEnabled((Button 2222)) }
    LogClick 2222
    Wait-Until 'previous page and paused follow' { (LogStatus).Contains('跟随已暂停') -and [ProxyUiSmoke]::IsWindowEnabled((Button 2206)) }
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
    Write-Output 'PASS: snapshots, pause/resume, view cursor, search/level, file tail, non-persistent apply/normal save, cancel/history/all cleanup and partial failure.'
    Write-Output 'LIMITS: synthetic messages; no physical keyboard or multi-monitor DPI validation; no real Startup writes.'
}
& (Join-Path $PSScriptRoot 'smoke-proxy-ui.ps1') -ExePath $ExePath -StepTimeoutSeconds $StepTimeoutSeconds -LogsOnly
