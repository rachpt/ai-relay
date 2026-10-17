# Local mock, synthetic credentials, own child/temp data only. Never touches Startup.
[CmdletBinding()]
param([string]$ExePath, [ValidateRange(5,30)][int]$StepTimeoutSeconds = 20)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $ExePath) { $ExePath = Join-Path $PSScriptRoot '..\target\debug\ai-relay.exe' }
$exe = (Get-Item -LiteralPath $ExePath).FullName
$source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'smoke-proxy-ui.ps1'))
$match = [regex]::Match($source, "(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@")
if (-not $match.Success) { throw 'Missing PID-checked smoke helpers' }
Add-Type -TypeDefinition $match.Groups[1].Value
Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Net;
public static class HistoryRequest {
    public static void Get(int port, string token) {
        var r=(HttpWebRequest)WebRequest.Create("http://127.0.0.1:"+port+"/usage-json");
        r.Proxy=null; r.Timeout=3000; r.ReadWriteTimeout=3000;
        r.Headers["Authorization"]="Bearer "+token;
        using(var response=r.GetResponse()) using(var reader=new StreamReader(response.GetResponseStream())) {
            if(!reader.ReadToEnd().Contains("local-usage-model")) throw new Exception("Mock response mismatch");
        }
    }
}
'@
$temp = Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-history-ui-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temp)
$child=$null; $mock=$null; $window=[IntPtr]::Zero
$overall=[Diagnostics.Stopwatch]::StartNew()
function Wait([string]$Name,[scriptblock]$Test) {
    $timer=[Diagnostics.Stopwatch]::StartNew()
    while($timer.Elapsed.TotalSeconds -lt $StepTimeoutSeconds -and $overall.Elapsed.TotalSeconds -lt 180) {
        if($child.HasExited) { throw "Child exited: $Name" }
        if(& $Test) { return }
        Start-Sleep -Milliseconds 60
    }
    throw ("Timeout: $Name; status=" + (Text 2325))
}
function Control([int]$Id) { [ProxyUiSmoke]::GetDlgItem($window,$Id) }
function Text([int]$Id) { [ProxyUiSmoke]::ReadText((Control $Id),[uint32]$child.Id) }
function Set-Field([int]$Id,[string]$Value) { [ProxyUiSmoke]::SetText((Control $Id),[uint32]$child.Id,$Value) }
function Click([int]$Id) {
    if(-not [ProxyUiSmoke]::IsWindowEnabled((Control $Id))) { throw "Disabled control $Id" }
    [ProxyUiSmoke]::Post((Control $Id),[uint32]$child.Id,0xF5,0)
}
function Combo([int]$Id,[uint32]$Index) {
    [ProxyUiSmoke]::Send((Control $Id),[uint32]$child.Id,0x14E,$Index)
    [ProxyUiSmoke]::Send($window,[uint32]$child.Id,0x111,($Id -bor (1 -shl 16)))
}
function Answer-No {
    $script:dialog=[IntPtr]::Zero
    Wait 'cancel-only confirmation' { $script:dialog=[ProxyUiSmoke]::Dialog([uint32]$child.Id); $dialog -ne [IntPtr]::Zero }
    $answered=$dialog
    [ProxyUiSmoke]::Post([ProxyUiSmoke]::GetDlgItem($dialog,7),[uint32]$child.Id,0xF5,0)
    Wait 'cancelled dialog closed' { [ProxyUiSmoke]::Dialog([uint32]$child.Id) -ne $answered }
}
function Query([int]$Count) {
    Click 2320
    Start-Sleep -Milliseconds 250
    Wait "history result $Count" { (Text 2325).Contains("匹配 $Count 请求") -and [ProxyUiSmoke]::IsWindowEnabled((Control 2320)) }
}
function StartGui {
    $script:child=Start-Process -FilePath $exe -ArgumentList @('--config-dir',('"'+$temp+'"')) -PassThru
    Wait 'window loaded' { $script:window=[ProxyUiSmoke]::Find([uint32]$child.Id); $window -ne [IntPtr]::Zero -and [ProxyUiSmoke]::IsWindowEnabled((Control 2003)) }
    [ProxyUiSmoke]::SelectTab($window,[uint32]$child.Id,1)
    Wait 'history scope received from controller' { (Text 2330).Contains('数据目录已变化') }
}
function StopGui {
    [ProxyUiSmoke]::Post($window,[uint32]$child.Id,0x111,1002)
    if(-not $child.WaitForExit(38000)) { throw 'Graceful exit timeout' }
    if($child.ExitCode -ne 0) { throw 'Nonzero child exit' }
    $child.Dispose();$script:child=$null
}
try {
    $mock=[ProxyUiMock]::new('history-local-mock')
    $port=[ProxyUiSmoke]::FreePort()
    while($port -eq $mock.Port) { $port=[ProxyUiSmoke]::FreePort() }
    [IO.File]::WriteAllText((Join-Path $temp 'config.toml'), "schema_version = 1`n[upstream]`nurl = 'http://127.0.0.1:$($mock.Port)/v1'`n[listen]`naddress = '127.0.0.1'`nport = $port`n[proxy]`nauto_start = true`n")
    StartGui
    $a='synthetic-client-a-'+[Guid]::NewGuid().ToString('N')
    $b='synthetic-client-b-'+[Guid]::NewGuid().ToString('N')
    for($i=0;$i -lt 52;$i++) { [HistoryRequest]::Get($port,$a) }
    [HistoryRequest]::Get($port,$b)
    Query 53
    if(-not (Text 2330).Contains('总 795')) { throw 'True token summary mismatch' }
    if(-not [ProxyUiSmoke]::IsWindowVisible((Control 2332))) { throw 'Native chart not visible' }
    $detail=Text 2331
    $ids=[regex]::Matches($detail,'[0-9a-f]{64}')
    if($ids.Count -eq 0) { throw 'No opaque credential identity in real details' }
    $id=$ids[0].Value
    Set-Field 2307 $id
    # Newest record is b: query ordering determines which actual identity is copied.
    Click 2320
    Start-Sleep -Milliseconds 250
    Wait 'credential-filtered query' { [ProxyUiSmoke]::IsWindowEnabled((Control 2320)) -and (Text 2325).Contains('历史已查询') }
    if(-not ((Text 2325).Contains('匹配 1 请求') -or (Text 2325).Contains('匹配 52 请求'))) { throw ('Credential filter ignored: ' + (Text 2325) + '; ID=' + $id + '; draft=' + (Text 2307)) }
    Set-Field 2307 ''
    $upstreamId=[regex]::Match($detail,'\| ([0-9a-f]{64})\r?\n').Groups[1].Value
    if(-not $upstreamId) { throw 'Missing actual upstream credential identity' }
    Set-Field 2308 $upstreamId
    Click 2320
    Start-Sleep -Milliseconds 250
    Wait 'upstream-credential-filtered query' { [ProxyUiSmoke]::IsWindowEnabled((Control 2320)) -and (Text 2325).Contains('历史已查询') }
    if(-not ((Text 2325).Contains('匹配 1 请求') -or (Text 2325).Contains('匹配 52 请求'))) { throw 'Upstream credential filter ignored' }
    Set-Field 2308 ''
    Set-Field 2304 '127.0.0.1'; Set-Field 2305 'local-usage-model'
    Set-Field 2309 "http://127.0.0.1:$($mock.Port)"
    Combo 2310 1; Combo 2312 1; Set-Field 2313 '200'
    Combo 2314 1; Combo 2315 3; Combo 2316 5
    Query 53
    if(-not (Text 2330).Contains('分组')) { throw 'No real grouped summary' }
    Click 2322
    Wait 'second details page' { (Text 2325).Contains('offset 50 / 3 行') }
    Click 2321
    Wait 'first details page' { (Text 2325).Contains('offset 0 / 50 行') }
    Set-Field 2305 'no-such-local-model'; Query 0
    if(-not (Text 2330).Contains('总 未知')) { throw 'Missing token usage rendered as zero' }
    Set-Field 2305 'local-usage-model'; Query 53
    Combo 2301 1; Query 53
    Combo 2301 2; Query 53
    Combo 2301 3; Query 53
    Combo 2301 4
    Set-Field 2302 ([DateTime]::UtcNow.ToString('yyyy-MM-dd'))
    Set-Field 2303 ([DateTime]::UtcNow.AddDays(1).ToString('yyyy-MM-dd'))
    Query 53
    # Export uses the real confirmation and native save dialog on its worker thread.
    Click 2323
    $script:dialog=[IntPtr]::Zero
    Wait 'export range confirmation' { $script:dialog=[ProxyUiSmoke]::Dialog([uint32]$child.Id); $script:dialog -ne [IntPtr]::Zero }
    $confirmation=$dialog
    [ProxyUiSmoke]::Post([ProxyUiSmoke]::GetDlgItem($dialog,6),[uint32]$child.Id,0xF5,0)
    Wait 'native CSV save dialog' {
        $script:dialog=[ProxyUiSmoke]::Dialog([uint32]$child.Id)
        $dialog -ne [IntPtr]::Zero -and $dialog -ne $confirmation -and @([ProxyUiSmoke]::Children($dialog,'Edit') | Where-Object { [ProxyUiSmoke]::IsWindowVisible($_) }).Count -gt 0
    }
    $csv=Join-Path $temp 'current-page.csv'
    $pathEdit=@([ProxyUiSmoke]::Children($dialog,'Edit') | Where-Object { [ProxyUiSmoke]::IsWindowVisible($_) })[-1]
    [ProxyUiSmoke]::SetText($pathEdit,[uint32]$child.Id,$csv)
    [ProxyUiSmoke]::Post([ProxyUiSmoke]::GetDlgItem($dialog,1),[uint32]$child.Id,0xF5,0)
    Wait 'CSV written by history worker' { (Test-Path -LiteralPath $csv) -and (Text 2325).Contains('CSV 已导出') }
    $data=[IO.File]::ReadAllText($csv)
    if($data.Contains($a) -or $data.Contains($b) -or $data.Contains($id) -or $data.Contains('127.0.0.1') -or $data.Contains('credential')) { throw 'CSV identity leak' }
    if(@(Import-Csv -LiteralPath $csv).Count -ne 50) { throw 'CSV export did not use current bounded page' }
    # Cancelling confirmation must never enter the file chooser or write a file.
    Click 2323
    Answer-No
    Click 2324
    Wait 'real recording counters' { (Text 2325).Contains('历史记录状态') -and [ProxyUiSmoke]::IsWindowEnabled((Control 2320)) }
    Click 2317
    if((Text 2304) -ne '' -or (Text 2309) -ne '' -or -not (Text 2325).Contains('草稿已重置')) { throw 'Reset did not clear draft-only filters' }
    if(-not (Text 2330).Contains('总 795')) { throw 'Reset discarded applied history report' }
    Query 53
    Combo 2300 1
    Wait 'session subview visible' { [ProxyUiSmoke]::IsWindowVisible((Control 2020)) -and -not [ProxyUiSmoke]::IsWindowVisible((Control 2330)) }
    Combo 2300 0
    if(-not (Text 2330).Contains('总 795')) { throw 'Session refresh overwrote history' }
    StopGui
    StartGui
    Query 53
    if(-not (Text 2330).Contains('总 795')) { throw 'Persisted totals lost after restart' }
    StopGui
    Write-Output 'PASS: 53 real local requests, both credential filters, combined filters, UTC ranges, groups, paging, chart visibility, unknown values, worker CSV redaction/cancel, counters/reset, session separation and restart persistence.'
    Write-Output 'LIMITS: synthetic messages; no real Startup changes, Explorer launch, physical input or multi-monitor chart visual acceptance.'
} finally {
    if($null -ne $child) { if(-not $child.HasExited) { $child.Kill(); [void]$child.WaitForExit(10000) }; $child.Dispose() }
    if($null -ne $mock) { $mock.Dispose() }
    if(Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
}
