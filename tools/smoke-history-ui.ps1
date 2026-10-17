# Local mock, synthetic credentials, own child/temp data only. Never touches Startup.
[CmdletBinding()]
param([string]$ExePath, [ValidateRange(5,30)][int]$StepTimeoutSeconds = 20)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# A different config directory does NOT bypass the user/session singleton.
# Check only; never send quit/stop to, or kill, a pre-existing main instance.
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$session = [Diagnostics.Process]::GetCurrentProcess().SessionId
$instanceName = "Local\AIRelay.$sid.$session.v1"
try { $existing = [Threading.Mutex]::OpenExisting($instanceName) } catch [Threading.WaitHandleCannotBeOpenedException] { $existing = $null }
if ($null -ne $existing) { $existing.Dispose(); throw 'Existing application instance: history GUI smoke skipped to protect user state.' }
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
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading.Tasks;
public static class HistoryUi {
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    public static void CheckOwner(IntPtr h,uint expected) {
        uint pid; GetWindowThreadProcessId(h,out pid);
        if(h==IntPtr.Zero || pid!=expected) throw new Exception("Refusing to access another process or missing history control");
    }
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr h, uint m, UIntPtr w, IntPtr l, uint f, uint t, out UIntPtr result);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr h, uint m, UIntPtr w, StringBuilder l, uint f, uint t, out UIntPtr result);
    public static int Value(IntPtr h, uint pid, uint message) {
        CheckOwner(h,pid); UIntPtr result;
        if(SendMessageTimeoutW(h,message,UIntPtr.Zero,IntPtr.Zero,2,2000,out result)==IntPtr.Zero) throw new Exception("Scalar control query failed");
        return unchecked((int)result.ToUInt64());
    }
    public static string Text(IntPtr h, uint pid) {
        CheckOwner(h,pid); UIntPtr result; var text=new StringBuilder(262144);
        if(SendMessageTimeoutW(h,13,new UIntPtr((uint)text.Capacity),text,2,2000,out result)==IntPtr.Zero) throw new Exception("Text read failed");
        return text.ToString();
    }
}
public sealed class HistoryMock : IDisposable {
    const string UsageJson = "{\"model\":\"local-usage-model\",\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}";
    readonly TcpListener listener;
    readonly Task task;
    volatile bool stopping;
    public int Port { get; private set; }
    public HistoryMock() {
        listener=new TcpListener(IPAddress.Loopback,0); listener.Start();
        Port=((IPEndPoint)listener.LocalEndpoint).Port;
        task=Task.Run(async delegate {
            try {
                while(!stopping) {
                    using(var client=await listener.AcceptTcpClientAsync()) {
                        client.ReceiveTimeout=3000; client.SendTimeout=3000;
                        using(var stream=client.GetStream())
                        using(var reader=new StreamReader(stream,Encoding.UTF8,false,1024,true)) {
                            string first=reader.ReadLine(), line; int length=0, headers=0;
                            while(!String.IsNullOrEmpty(line=reader.ReadLine())) {
                                headers+=line.Length; if(headers>16384) throw new Exception("Mock header limit");
                                if(line.StartsWith("Content-Length:",StringComparison.OrdinalIgnoreCase)) length=int.Parse(line.Substring(15).Trim());
                            }
                            if(length<0 || length>4096) throw new Exception("Mock body limit");
                            var body=new char[length]; int read=0;
                            while(read<length) { int n=reader.Read(body,read,length-read); if(n==0) throw new Exception("Short mock body"); read+=n; }
                            bool valid=first=="GET /v1/usage-json HTTP/1.1" || first=="POST /v1/chat/completions HTTP/1.1";
                            string response=valid?UsageJson:"bad mock target";
                            byte[] bytes=Encoding.UTF8.GetBytes("HTTP/1.1 "+(valid?"200 OK":"400 Bad Request")+"\r\nContent-Type: application/json\r\nContent-Length: "+Encoding.UTF8.GetByteCount(response)+"\r\nConnection: close\r\n\r\n"+response);
                            stream.Write(bytes,0,bytes.Length);
                        }
                    }
                }
            } catch(ObjectDisposedException) { if(!stopping) throw; }
              catch(SocketException) { if(!stopping) throw; }
        });
    }
    public void Dispose() { stopping=true; listener.Stop(); if(!task.Wait(5000)) throw new Exception("Mock shutdown timeout"); }
}
public static class HistoryRequest {
    public static void Get(int port, string token) { Send(port,token,false); }
    public static void Post(int port, string token) { Send(port,token,true); }
    static void Send(int port, string token, bool post) {
        var r=(HttpWebRequest)WebRequest.Create("http://127.0.0.1:"+port+(post?"/chat/completions":"/usage-json"));
        r.Proxy=null; r.Timeout=3000; r.ReadWriteTimeout=3000;
        r.Headers["Authorization"]="Bearer "+token;
        if(post) {
            r.Method="POST"; r.ContentType="application/json";
            byte[] data=Encoding.UTF8.GetBytes("{\"model\":\"local-requested-model\",\"messages\":[]}");
            r.ContentLength=data.Length;
            using(var body=r.GetRequestStream()) body.Write(data,0,data.Length);
        }
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
$script:case='startup'
$script:lastAction='none'
function Trace-Action([string]$Action) {
    $caller=(Get-PSCallStack)[2]
    $script:lastAction="$Action at $($caller.Location)"
    $script:case=$script:lastAction
    Write-Output ("STEP {0}: {1}" -f $overall.Elapsed.ToString('mm\:ss\.fff'),$script:lastAction)
}
function Wait([string]$Name,[scriptblock]$Test) {
    $script:case=$Name
    $timer=[Diagnostics.Stopwatch]::StartNew()
    while($timer.Elapsed.TotalSeconds -lt $StepTimeoutSeconds -and $overall.Elapsed.TotalSeconds -lt 300) {
        if($child.HasExited) { throw "Child exited: $Name" }
        if(& $Test) { return }
        Start-Sleep -Milliseconds 60
    }
    throw ("Timeout: $Name; lastAction=$script:lastAction; status=" + (Text 2325))
}
function Control([int]$Id) { [ProxyUiSmoke]::GetDlgItem($window,$Id) }
function Text([int]$Id) { [HistoryUi]::Text((Control $Id),[uint32]$child.Id) }
function Rows { [HistoryUi]::Value((Control 2337),[uint32]$child.Id,0x1004) }
function Operation {
    $values=@{}
    foreach($part in (Text 2354).Split(';')) {
        $pair=$part.Split('=')
        if($pair.Count -eq 2) { $values[$pair[0]]=[uint64]$pair[1] }
    }
    if($values.Count -ne 5) { throw 'Missing history operation state; build the current source before smoke' }
    return $values
}
function Ready {
    $state=Operation
    $state.idle -eq 1 -and [ProxyUiSmoke]::IsWindowEnabled((Control 2320)) -and [ProxyUiSmoke]::Dialog([uint32]$child.Id) -eq [IntPtr]::Zero
}
function Fence {
    # SendMessage can overtake posted choice notifications/BM_CLICK. A posted
    # owner-thread acknowledgement observes their effects, even for no-op choices.
    $before=(Operation).fence
    [ProxyUiSmoke]::Post($window,[uint32]$child.Id,0x111,2397)
    Wait 'history owner-thread queue fence' { (Operation).fence -gt $before }
}
function Wait-Idle {
    Fence
    Wait 'history truly idle' { Ready }
    if((Operation).rejected -ne 0) { throw ('History rejected an action; state=' + (Text 2354) + '; status=' + (Text 2325)) }
}
function Set-Field([int]$Id,[string]$Value) {
    Wait-Idle
    if(-not [ProxyUiSmoke]::IsWindowEnabled((Control $Id))) { throw "Disabled field $Id" }
    [ProxyUiSmoke]::SetText((Control $Id),[uint32]$child.Id,$Value)
}
function Click([int]$Id) {
    Trace-Action "Click $Id"
    Wait-Idle
    if(-not [ProxyUiSmoke]::IsWindowEnabled((Control $Id))) { throw "Disabled control $Id; case=$script:case; lastAction=$script:lastAction; status=$(Text 2325)" }
    [ProxyUiSmoke]::Post((Control $Id),[uint32]$child.Id,0xF5,0)
    Fence
}
function Combo([int]$Id,[uint32]$Index) {
    Trace-Action "Combo $Id index $Index"
    Wait-Idle
    $before=(Operation).sequence
    [ProxyUiSmoke]::Send((Control $Id),[uint32]$child.Id,0x14E,$Index)
    if([HistoryUi]::Value((Control $Id),[uint32]$child.Id,0x147) -ne $Index) { throw "CB_SETCURSEL/CB_GETCURSEL mismatch: $Id index $Index" }
    [ProxyUiSmoke]::Send($window,[uint32]$child.Id,0x111,($Id -bor (1 -shl 16)))
    Wait-Idle
    if($Id -in @(2333,2334,2335,2336) -and (Operation).sequence -ne ($before+1)) { throw "Choice $Id did not complete exactly one automatic query" }
}
function Choice-Key([int]$Id,[uint32]$Key,[int]$ExpectedIndex) {
    Trace-Action "Native choice key $Id key $Key"
    Wait-Idle
    $before=(Operation).sequence
    # Real native Tab keyboard input produces TCN_SELCHANGE, then choices::notify
    # posts CBN_SELCHANGE. Do not synthesize WM_COMMAND on this path.
    [ProxyUiSmoke]::Send((Control $Id),[uint32]$child.Id,0x100,$Key)
    Wait-Idle
    if([HistoryUi]::Value((Control $Id),[uint32]$child.Id,0x147) -ne $ExpectedIndex) { throw 'Native choice keyboard selection mismatch' }
    if((Operation).sequence -ne ($before+1)) { throw 'Posted native choice notification did not complete exactly one query' }
}
function Assert-Result([int]$Count) {
    Wait-Idle
    Wait "history result $Count" { (Ready) -and (Text 2325).Contains("匹配 $Count 请求") }
}
function Answer([bool]$Yes) {
    $script:dialog=[IntPtr]::Zero
    Wait 'confirmation' { $script:dialog=[ProxyUiSmoke]::Dialog([uint32]$child.Id); $dialog -ne [IntPtr]::Zero }
    $answered=$dialog
    $button=7; if($Yes) { $button=6 }
    [ProxyUiSmoke]::Post([ProxyUiSmoke]::GetDlgItem($dialog,$button),[uint32]$child.Id,0xF5,0)
    Wait 'confirmation closed' { [ProxyUiSmoke]::Dialog([uint32]$child.Id) -ne $answered }
    Wait-Idle
}
function Answer-No { Answer $false }
function Query([int]$Count) {
    Click 2320
    Assert-Result $Count
}
function Save-Export([string]$Path,[int]$Count,[bool]$All) {
    $choice=0; if($All) { $choice=1 }
    Combo 2353 $choice
    Click 2323
    $script:dialog=[IntPtr]::Zero
    Wait 'export privacy confirmation' { $script:dialog=[ProxyUiSmoke]::Dialog([uint32]$child.Id); $dialog -ne [IntPtr]::Zero }
    $confirmation=$dialog
    [ProxyUiSmoke]::Post([ProxyUiSmoke]::GetDlgItem($dialog,6),[uint32]$child.Id,0xF5,0)
    Wait 'native CSV save dialog' {
        $script:dialog=[ProxyUiSmoke]::Dialog([uint32]$child.Id)
        $dialog -ne [IntPtr]::Zero -and $dialog -ne $confirmation -and @([ProxyUiSmoke]::Children($dialog,'Edit') | Where-Object { [ProxyUiSmoke]::IsWindowVisible($_) }).Count -gt 0
    }
    $pathEdit=@([ProxyUiSmoke]::Children($dialog,'Edit') | Where-Object { [ProxyUiSmoke]::IsWindowVisible($_) })[-1]
    [ProxyUiSmoke]::SetText($pathEdit,[uint32]$child.Id,$Path)
    [ProxyUiSmoke]::Post([ProxyUiSmoke]::GetDlgItem($dialog,1),[uint32]$child.Id,0xF5,0)
    Wait 'worker CSV result' { (Test-Path -LiteralPath $Path) -and (Text 2325).Contains('CSV 已导出') -and (Ready) }
    if(@(Import-Csv -LiteralPath $Path).Count -ne $Count) { throw 'CSV range/row count mismatch' }
    $data=[IO.File]::ReadAllText($Path)
    if($data.Contains($a) -or $data.Contains($b) -or $data.Contains($id)) { throw 'CSV raw credential or HMAC leak' }
    if((Text 2325).Contains('仅当前页 true') -eq $All) { throw 'CSV result range label mismatch' }
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
    $mock=[HistoryMock]::new()
    $port=[ProxyUiSmoke]::FreePort()
    while($port -eq $mock.Port) { $port=[ProxyUiSmoke]::FreePort() }
    [IO.File]::WriteAllText((Join-Path $temp 'config.toml'), "schema_version = 1`n[upstream]`nurl = 'http://127.0.0.1:$($mock.Port)/v1'`n[listen]`naddress = '127.0.0.1'`nport = $port`n[proxy]`nauto_start = true`n")
    StartGui
    $a='synthetic-client-a-'+[Guid]::NewGuid().ToString('N')
    $b='synthetic-client-b-'+[Guid]::NewGuid().ToString('N')
    for($i=0;$i -lt 52;$i++) { [HistoryRequest]::Get($port,$a) }
    [HistoryRequest]::Post($port,$b)
    Query 53
    if(-not (Text 2330).Contains('总 795')) { throw 'True token summary mismatch' }
    if(-not [ProxyUiSmoke]::IsWindowVisible((Control 2332))) { throw 'Native chart not visible' }
    if([ProxyUiSmoke]::Class((Control 2337)) -ne 'SysListView32' -or (Rows) -ne 50 -or -not [ProxyUiSmoke]::IsWindowVisible((Control 2337))) { throw 'Native details table absent/empty' }
    if([ProxyUiSmoke]::IsWindowVisible((Control 2331))) { throw 'Legacy text overlaps native table' }
    if(-not (Text 2331).Contains('local-requested-model')) { throw 'Actual request model not collected' }
    Click 2318 # Expanded filters replace the chart, not the native table.
    Wait 'advanced filters without table overlap' { [ProxyUiSmoke]::IsWindowVisible((Control 2307)) -and -not [ProxyUiSmoke]::IsWindowVisible((Control 2332)) -and [ProxyUiSmoke]::IsWindowVisible((Control 2337)) }
    $detail=Text 2331
    $ids=[regex]::Matches($detail,'[0-9a-f]{64}')
    if($ids.Count -eq 0) { throw 'No opaque credential identity in real details' }
    $id=$ids[0].Value
    Set-Field 2307 $id
    # Newest record is b: query ordering determines which actual identity is copied.
    Click 2320
    Wait 'credential-filtered query' { [ProxyUiSmoke]::IsWindowEnabled((Control 2320)) -and (Text 2325).Contains('历史已查询') }
    if(-not ((Text 2325).Contains('匹配 1 请求') -or (Text 2325).Contains('匹配 52 请求'))) { throw ('Credential filter ignored: ' + (Text 2325) + '; ID=' + $id + '; draft=' + (Text 2307)) }
    Set-Field 2307 ''
    $upstreamId=[regex]::Match($detail,'\| ([0-9a-f]{64})\r?\n').Groups[1].Value
    if(-not $upstreamId) { throw 'Missing actual upstream credential identity' }
    Set-Field 2308 $upstreamId
    Click 2320
    Wait 'upstream-credential-filtered query' { [ProxyUiSmoke]::IsWindowEnabled((Control 2320)) -and (Text 2325).Contains('历史已查询') }
    if(-not ((Text 2325).Contains('匹配 1 请求') -or (Text 2325).Contains('匹配 52 请求'))) { throw 'Upstream credential filter ignored' }
    Set-Field 2308 ''
    Set-Field 2304 '127.0.0.1'; Set-Field 2305 'local-usage-model'
    Set-Field 2309 "http://127.0.0.1:$($mock.Port)"
    Combo 2310 0; Combo 2312 1; Set-Field 2313 '200'
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
    # Request-model + new dimensions, timezone/granularity and backend sorting.
    Set-Field 2306 'local-requested-model'; Query 1
    Set-Field 2306 ''; Query 53
    Combo 2301 0
    Choice-Key 2333 0x27 1
    Assert-Result 53
    if(-not (Text 2330).Contains('时区 Local')) { throw 'Native timezone notification did not apply Local query' }
    Choice-Key 2333 0x25 0
    Assert-Result 53
    Choice-Key 2334 0x27 1
    Assert-Result 53
    if(-not (Text 2330).Contains('趋势 Hour')) { throw 'Native granularity notification did not apply Hour query' }
    Choice-Key 2334 0x25 0
    Assert-Result 53
    foreach($zone in @(0,1)) {
        Combo 2333 $zone
        Combo 2301 1; Query 53
        Combo 2301 4
        $now=[DateTime]::UtcNow; if($zone -eq 1) { $now=[DateTime]::Now }
        Set-Field 2302 ($now.ToString('yyyy-MM-dd'))
        Set-Field 2303 ($now.AddDays(1).ToString('yyyy-MM-dd'))
        Query 53
        Combo 2301 0
        foreach($grain in @(0,1,2)) {
            Combo 2334 $grain; Assert-Result 53
            if(-not (Text 2330).Contains('趋势')) { throw 'No backend trend labels' }
        }
    }
    Combo 2333 0; Combo 2301 0
    Combo 2314 9; Combo 2315 10; Combo 2316 11; Query 53
    Combo 2338 1
    if((Rows) -ne 2 -or -not [ProxyUiSmoke]::IsWindowVisible((Control 2337))) { throw 'Grouped native table mismatch' }
    for($sort=0;$sort -lt 8;$sort++) {
        Combo 2336 $sort
        Wait 'group sort backend result' { (Ready) -and (Text 2325).Contains('匹配 53 请求') }
        if((Rows) -ne 2) { throw 'Sorting changed real aggregate groups' }
    }
    Combo 2338 0
    for($sort=0;$sort -lt 4;$sort++) {
        Combo 2335 $sort
        Wait 'details sort backend result' { (Ready) -and (Text 2325).Contains('匹配 53 请求') }
        if((Rows) -ne 50) { throw 'Details sorting lost rows' }
    }
    Combo 2338 2
    if(-not [ProxyUiSmoke]::IsWindowVisible((Control 2330)) -or [ProxyUiSmoke]::IsWindowVisible((Control 2337))) { throw 'Accessible summary view overlaps table' }
    Combo 2338 3
    if(-not [ProxyUiSmoke]::IsWindowVisible((Control 2331))) { throw 'Accessible metadata text unavailable' }
    Combo 2338 0
    Save-Export (Join-Path $temp 'current-page.csv') 50 $false
    Save-Export (Join-Path $temp 'all-matches.csv') 53 $true
    $allRows=@(Import-Csv -LiteralPath (Join-Path $temp 'all-matches.csv'))
    if(@($allRows | Where-Object { $_.requested_model -eq 'local-requested-model' }).Count -ne 1) { throw 'All export omitted real request model' }
    if($allRows[0].PSObject.Properties.Name -contains 'client' -or $allRows[0].PSObject.Properties.Name -contains 'ip') { throw 'All export exposes private identity columns' }
    # Cancelling confirmation must never enter the file chooser or write a file.
    Click 2323
    Answer-No
    Click 2324
    Wait 'real recording counters' { (Text 2325).Contains('历史记录状态') -and [ProxyUiSmoke]::IsWindowEnabled((Control 2320)) }
    Click 2317
    Wait 'draft reset completed' { (Text 2325).Contains('草稿已重置') }
    if((Text 2304) -ne '' -or (Text 2309) -ne '' -or -not (Text 2325).Contains('草稿已重置')) { throw 'Reset did not clear draft-only filters' }
    if(-not (Text 2330).Contains('总 795')) { throw 'Reset discarded applied history report' }
    Query 53
    Click 2339
    Wait 'storage stats' { (Ready) -and (Text 2351).Contains('53 条记录') }
    if([ProxyUiSmoke]::IsWindowVisible((Control 2337))) { throw 'Management overlaps table' }
    Set-Field 2341 $id; Set-Field 2342 'smoke-client-label'; Click 2343
    Wait 'alias save and report refresh' { (Ready) -and (Text 2331).Contains('smoke-client-label') }
    if(-not (Text 2330).Contains('总 795')) { throw 'Alias changed real usage' }
    Click 2344
    Wait 'alias removal and refresh' { (Ready) -and -not (Text 2331).Contains('smoke-client-label') }
    Set-Field 2342 'smoke-client-label'; Click 2343
    Wait 'alias restored for persistence' { (Ready) -and (Text 2331).Contains('smoke-client-label') }
    Set-Field 2342 'sk-synthetic-not-a-real-key'; Click 2343
    Wait 'secret-like alias rejected' { (Text 2351).Contains('请勿粘贴密钥') }
    Set-Field 2342 'smoke-client-label'
    Set-Field 2346 '90'; Set-Field 2347 '60'
    Click 2348; Answer $false
    Click 2345
    Wait 'policy cancel preserved real policy' { (Ready) -and (Text 2347) -eq '100000' }
    Set-Field 2346 '90'; Set-Field 2347 '60'
    Click 2348; Answer $true
    Wait 'policy saved and applied' { (Ready) -and (Text 2325).Contains('历史策略已保存并应用') }
    if(-not [IO.File]::ReadAllText((Join-Path $temp 'config.toml')).Contains('max_records = 60')) { throw 'Policy was not persisted' }
    Click 2350
    Wait 'backend clear preview' { (Ready) -and (Text 2351).Contains('53 条') -and [ProxyUiSmoke]::IsWindowEnabled((Control 2352)) }
    Click 2352; Answer $false
    Click 2345
    Wait 'clear cancellation kept records' { (Ready) -and (Text 2351).Contains('53 条记录') }
    Click 2339
    Query 53
    Combo 2300 1
    Wait 'session subview visible' { [ProxyUiSmoke]::IsWindowVisible((Control 2020)) -and -not [ProxyUiSmoke]::IsWindowVisible((Control 2330)) }
    Combo 2300 0
    if(-not (Text 2330).Contains('总 795')) { throw 'Session refresh overwrote history' }
    StopGui
    StartGui
    Query 53
    if(-not (Text 2330).Contains('总 795')) { throw 'Persisted totals lost after restart' }
    if(-not (Text 2331).Contains('smoke-client-label')) { throw 'Alias persistence lost after restart' }
    Click 2339
    Wait 'persisted policy storage' { (Ready) -and (Text 2347) -eq '60' }
    Click 2350
    Wait 'clear preview after restart' { (Ready) -and [ProxyUiSmoke]::IsWindowEnabled((Control 2352)) }
    Click 2352; Answer $true
    Wait 'clear completed' { (Ready) -and (Text 2325).Contains('历史已清空') }
    Click 2345
    Wait 'clear keeps aliases by default' { (Ready) -and (Text 2351).Contains('0 条记录') -and (Text 2351).Contains('1 个别名') }
    [HistoryRequest]::Post($port,$b)
    Click 2339; Query 1
    Click 2339
    Wait 'post-clear recording continued' { (Ready) -and (Text 2351).Contains('1 条记录') }
    Click 2350
    Wait 'preview before checkbox change' { (Ready) -and [ProxyUiSmoke]::IsWindowEnabled((Control 2352)) }
    Click 2349
    Wait 'changed alias scope invalidates preview' { -not [ProxyUiSmoke]::IsWindowEnabled((Control 2352)) }
    Click 2350
    Wait 'explicit alias removal preview' { (Ready) -and (Text 2351).Contains('true') -and [ProxyUiSmoke]::IsWindowEnabled((Control 2352)) }
    Click 2352; Answer $true
    Wait 'records and aliases cleared' { (Ready) -and (Text 2325).Contains('历史已清空') }
    Click 2345
    Wait 'zero aliases verified by worker' { (Ready) -and (Text 2351).Contains('0 条记录') -and (Text 2351).Contains('0 个别名') }
    StopGui
    Write-Output 'PASS: queue-fenced idle with zero rejected commands, native asynchronous choice keyboard notifications, native details/group ListView, 53 local requests including POST model, credential/combined filters, UTC/local ranges, hour/day/month labels, backend sorting, paging, page/all worker CSV, alias CRUD/restart, policy confirmation/persistence, preview/cancel/clear, alias opt-in removal and continued recording.'
    Write-Output 'LIMITS: synthetic local metadata only; DST gap/fold checked by Rust injectable-zone unit tests, not by changing OS timezone. No real Startup changes, physical input, multi-monitor or screen-reader acceptance.'
} catch {
    Write-Output "FAIL: case=$script:case; lastAction=$script:lastAction"
    Write-Output $_.InvocationInfo.PositionMessage
    Write-Output $_.ScriptStackTrace
    if($window -ne [IntPtr]::Zero -and $null -ne $child -and -not $child.HasExited) {
        Write-Output ("HISTORY STATUS: " + (Text 2325))
        Write-Output ("OPERATION: " + (Text 2354))
        Write-Output ("QUERY ENABLED: " + [ProxyUiSmoke]::IsWindowEnabled((Control 2320)))
    }
    throw
} finally {
    if($null -ne $child) { if(-not $child.HasExited) { $child.Kill(); [void]$child.WaitForExit(10000) }; $child.Dispose() }
    if($null -ne $mock) { $mock.Dispose() }
    if(Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
}
