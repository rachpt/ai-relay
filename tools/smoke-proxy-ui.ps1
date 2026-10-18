# Interactive Windows desktop required. Uses only a new temporary --config-dir,
# loopback mock upstream and the exact child processes created here. No real keys.
[CmdletBinding()]
param(
    [string]$ExePath,
    [switch]$LogsOnly,
    [ValidateRange(5, 30)][int]$StepTimeoutSeconds = 15
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $PSScriptRoot '..\target\debug\ai-relay.exe'
}
$exe = (Get-Item -LiteralPath (Resolve-Path -LiteralPath $ExePath).Path).FullName

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
public static class ProxyUiSmoke {
    public delegate bool EnumProc(IntPtr hwnd, IntPtr parameter);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr parameter);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc callback, IntPtr parameter);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int length);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int length);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    public static void SelectTab(IntPtr window, uint pid, uint page) {
        var tab=GetDlgItem(window,2100); CheckOwner(tab,pid);
        // Let the native control generate its own in-process TCN_SELCHANGE.
        Send(tab,pid,0x100,0x24);
        for(uint i=0;i<page;i++) Send(tab,pid,0x100,0x27);
    }
    public static uint Selection(IntPtr edit, uint pid) {
        CheckOwner(edit,pid); UIntPtr result;
        if(SendMessageTimeoutW(edit,0xB0,UIntPtr.Zero,IntPtr.Zero,2,1000,out result)==IntPtr.Zero) throw new Exception("Selection read failed");
        return result.ToUInt32();
    }
    [DllImport("user32.dll", EntryPoint="GetDlgItem")] static extern IntPtr NativeGetDlgItem(IntPtr parent, int id);
    [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr window);
    public static IntPtr GetDlgItem(IntPtr parent, int id) {
        var found=NativeGetDlgItem(parent,id);
        if(found!=IntPtr.Zero) return found;
        EnumChildWindows(parent,delegate(IntPtr h, IntPtr p) { if(GetDlgCtrlID(h)==id) { found=h; return false; } return true; },IntPtr.Zero);
        return found;
    }
    [DllImport("user32.dll")] public static extern int GetWindowLongW(IntPtr hwnd, int index);
    [DllImport("user32.dll", SetLastError=true)] static extern bool PostMessageW(IntPtr hwnd, uint message, UIntPtr wp, IntPtr lp);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern IntPtr SendMessageTimeoutW(IntPtr hwnd, uint message, UIntPtr wp, IntPtr lp, uint flags, uint timeout, out UIntPtr result);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern IntPtr SendMessageTimeoutW(IntPtr hwnd, uint message, UIntPtr wp, string lp, uint flags, uint timeout, out UIntPtr result);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern IntPtr SendMessageTimeoutW(IntPtr hwnd, uint message, UIntPtr wp, StringBuilder lp, uint flags, uint timeout, out UIntPtr result);
    public static IntPtr Dialog(uint pid) {
        IntPtr found=IntPtr.Zero;
        EnumWindows(delegate(IntPtr h, IntPtr p) { if(Owner(h)==pid && Class(h)=="#32770") { found=h; return false; } return true; },IntPtr.Zero);
        return found;
    }
    public static uint Owner(IntPtr hwnd) { uint pid; GetWindowThreadProcessId(hwnd, out pid); return pid; }
    public static string Class(IntPtr hwnd) { var b = new StringBuilder(256); GetClassNameW(hwnd,b,b.Capacity); return b.ToString(); }
    public static string Title(IntPtr hwnd) { var b = new StringBuilder(2048); GetWindowTextW(hwnd,b,b.Capacity); return b.ToString(); }
    public static IntPtr Find(uint pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows(delegate(IntPtr h, IntPtr p) { if (Owner(h)==pid && Class(h)=="AIRelay.Window.v1") { found=h; return false; } return true; },IntPtr.Zero);
        return found;
    }
    public static IntPtr[] Children(IntPtr parent, string kind) {
        var found = new List<IntPtr>();
        EnumChildWindows(parent,delegate(IntPtr h, IntPtr p) { if(Class(h)==kind) found.Add(h); return true; },IntPtr.Zero);
        return found.ToArray();
    }
    public static void CheckOwner(IntPtr hwnd, uint pid) { if(hwnd==IntPtr.Zero || Owner(hwnd)!=pid) throw new Exception("Refusing to automate another process or missing control"); }
    public static void Send(IntPtr hwnd, uint pid, uint message, uint value) {
        CheckOwner(hwnd,pid); UIntPtr result;
        if(SendMessageTimeoutW(hwnd,message,new UIntPtr(value),IntPtr.Zero,2,1000,out result)==IntPtr.Zero) throw new Exception("Control message timed out/failed");
    }
    public static void SetText(IntPtr hwnd, uint pid, string text) {
        CheckOwner(hwnd,pid); UIntPtr result;
        if(SendMessageTimeoutW(hwnd,12,UIntPtr.Zero,text,2,1000,out result)==IntPtr.Zero || result==UIntPtr.Zero) throw new Exception("WM_SETTEXT failed");
    }
    public static string ReadText(IntPtr hwnd, uint pid) {
        CheckOwner(hwnd,pid); UIntPtr result; var b=new StringBuilder(2048);
        if(SendMessageTimeoutW(hwnd,13,new UIntPtr((uint)b.Capacity),b,2,1000,out result)==IntPtr.Zero) throw new Exception("WM_GETTEXT failed");
        return b.ToString();
    }
    public static void Post(IntPtr hwnd, uint pid, uint message, uint value) {
        CheckOwner(hwnd,pid);
        if(!PostMessageW(hwnd,message,new UIntPtr(value),IntPtr.Zero)) throw new Exception("PostMessage failed");
    }
    public static int FreePort() {
        var l=new TcpListener(IPAddress.Loopback,0); l.Start(); int port=((IPEndPoint)l.LocalEndpoint).Port; l.Stop(); return port;
    }
    public static bool PortClosed(int port) {
        using(var c=new TcpClient()) {
            var t=c.ConnectAsync(IPAddress.Loopback,port);
            try { if(!t.Wait(2500)) throw new Exception("Connection probe timed out (not proof of closed port)"); return !c.Connected; }
            catch(AggregateException e) { var s=e.InnerException as SocketException; if(s!=null && s.SocketErrorCode==SocketError.ConnectionRefused) return true; throw; }
        }
    }
    public static void Request(int port, string marker) { RequestBody(port, "/smoke?probe=1", marker); }
    public static string UsageJson = "{\"model\":\"local-usage-model\",\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15,\"prompt_tokens_details\":{\"cached_tokens\":3},\"completion_tokens_details\":{\"reasoning_tokens\":2}}}";
    public static string UsageSse = "data: "+UsageJson+"\n\ndata: [DONE]\n\n";
    public static void RequestBody(int port, string path, string marker) {
        var request=(HttpWebRequest)WebRequest.Create("http://127.0.0.1:"+port+path);
        request.Proxy=null; request.Timeout=2000; request.ReadWriteTimeout=2000;
        using(var response=(HttpWebResponse)request.GetResponse())
        using(var reader=new StreamReader(response.GetResponseStream())) {
            if(response.StatusCode!=HttpStatusCode.OK || reader.ReadToEnd()!=marker) throw new Exception("Unexpected proxy status/body");
        }
    }
}
public sealed class ProxyUiMock : IDisposable {
    readonly TcpListener listener;
    readonly Task task;
    readonly string marker;
    volatile bool stopping;
    int requests;
    int proxyCredentials;
    public int Requests { get { return Volatile.Read(ref requests); } }
    public int ProxyCredentialsReceived { get { return Volatile.Read(ref proxyCredentials); } }
    public int Port { get; private set; }
    public ProxyUiMock(string marker) {
        this.marker=marker; listener=new TcpListener(IPAddress.Loopback,0); listener.Start();
        Port=((IPEndPoint)listener.LocalEndpoint).Port;
        task=Task.Run(async delegate {
            try {
                while(!stopping) {
                    using(var client=await listener.AcceptTcpClientAsync()) {
                        client.ReceiveTimeout=2000; client.SendTimeout=2000;
                        using(var stream=client.GetStream())
                        using(var reader=new StreamReader(stream,Encoding.ASCII,false,1024,true)) {
                            string first=reader.ReadLine(); int bytes=0; string line;
                            while(!String.IsNullOrEmpty(line=reader.ReadLine())) {
                                bytes+=line.Length; if(bytes>16384) throw new Exception("Mock headers exceeded limit");
                                if(line.StartsWith("Proxy-Authorization:",StringComparison.OrdinalIgnoreCase)) Interlocked.Increment(ref proxyCredentials);
                            }
                            bool json=first=="GET /v1/usage-json HTTP/1.1";
                            bool sse=first=="GET /v1/usage-sse HTTP/1.1";
                            bool valid=json || sse || first=="GET /v1/smoke?probe=1 HTTP/1.1";
                            string body=json?ProxyUiSmoke.UsageJson:sse?ProxyUiSmoke.UsageSse:valid?marker:"bad target";
                            string contentType=json?"application/json":sse?"text/event-stream":"text/plain";
                            byte[] data=Encoding.ASCII.GetBytes("HTTP/1.1 "+(valid?"200 OK":"400 Bad Request")+"\r\nContent-Type: "+contentType+"\r\nContent-Length: "+body.Length+"\r\nConnection: close\r\n\r\n"+body);
                            stream.Write(data,0,data.Length);
                            if(valid) Interlocked.Increment(ref requests);
                        }
                    }
                }
            } catch(ObjectDisposedException) { if(!stopping) throw; }
              catch(SocketException) { if(!stopping) throw; }
        });
    }
    public void Dispose() { stopping=true; listener.Stop(); if(!task.Wait(3000)) throw new Exception("Mock shutdown timed out"); }
}
'@

$child = $null
$mock = $null
$temp = $null
$window = [IntPtr]::Zero
$overall = [Diagnostics.Stopwatch]::StartNew()
function Wait-Until([string]$Description, [scriptblock]$Condition) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ($timer.Elapsed.TotalSeconds -lt $StepTimeoutSeconds -and $overall.Elapsed.TotalSeconds -lt 150) {
        if (& $Condition) { return }
        if ($null -ne $child -and $child.HasExited) { throw "Child exited early: $Description (code $($child.ExitCode))" }
        Start-Sleep -Milliseconds 50
    }
    throw "Timed out: $Description"
}
function Start-TestGui {
    $script:child = Start-Process -FilePath $exe -ArgumentList @('--config-dir', ('"' + $temp + '"')) -PassThru
    Wait-Until 'native GUI window' {
        $script:window = [ProxyUiSmoke]::Find([uint32]$child.Id)
        $window -ne [IntPtr]::Zero
    }
}
function Button([int]$Id) { [ProxyUiSmoke]::GetDlgItem($window, $Id) }
function Select-Page([uint32]$Page) { [ProxyUiSmoke]::SelectTab($window, [uint32]$child.Id, $Page) }
function Click([int]$Id) {
    if ($Id -in @(2002, 2003)) { Select-Page 0; $Id += 100 }
    elseif ($Id -in @(2001, 2004)) { Select-Page 3 }
    $button = Button $Id
    if (-not [ProxyUiSmoke]::IsWindowEnabled($button)) { throw "Button $Id is disabled" }
    # BM_CLICK runs the real BUTTON procedure and generates BN_CLICKED.
    [ProxyUiSmoke]::Post($button, [uint32]$child.Id, 0xF5, 0)
}
function Usage {
    Select-Page 1
    [ProxyUiSmoke]::Send((Button 2300), [uint32]$child.Id, 0x14E, 1)
    [ProxyUiSmoke]::Send($window, [uint32]$child.Id, 0x111, (2300 -bor (1 -shl 16)))
    [ProxyUiSmoke]::ReadText((Button 2020), [uint32]$child.Id)
}
function Status {
    foreach ($control in [ProxyUiSmoke]::Children($window, 'Static')) {
        $caption = [ProxyUiSmoke]::Title($control)
        if ($caption.StartsWith('代理状态：')) { return $caption }
    }
    return ''
}
function Exit-TestGui {
    # Same command as tray Exit, not a physical Explorer tray-menu selection.
    [ProxyUiSmoke]::Post($window, [uint32]$child.Id, 0x111, 1002)
    if (-not $child.WaitForExit(38000)) { throw 'GUI graceful exit exceeded 38 seconds' }
    if ($child.ExitCode -ne 0) { throw "GUI exit code: $($child.ExitCode)" }
    $child.Dispose()
    $script:child = $null
}
try {
    $temp = Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-ui-smoke-' + [Guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($temp)
    $marker = 'mock-' + [Guid]::NewGuid().ToString('N')
    $mock = [ProxyUiMock]::new($marker)
    $port = [ProxyUiSmoke]::FreePort() # Reservation released; binding conflicts fail the test rather than falling back.
    while ($port -eq $mock.Port) { $port = [ProxyUiSmoke]::FreePort() }
    $configPath = Join-Path $temp 'config.toml'
    Start-TestGui
    Wait-Until 'initial configuration loaded and Save enabled' { [ProxyUiSmoke]::IsWindowEnabled((Button 2001)) }
    # Native segmented choices also use Tab controls; identify main navigation by ID.
    $tabs = @([ProxyUiSmoke]::Children($window, 'SysTabControl32') | Where-Object { $_ -eq (Button 2100) })
    if ($tabs.Count -ne 1) { throw 'Expected main native Common Controls Tab ID 2100' }
    Wait-Until 'initial overview page visible' { [ProxyUiSmoke]::IsWindowVisible((Button 2102)) -and -not [ProxyUiSmoke]::IsWindowVisible((Button 2001)) }
    Select-Page 3
    $edits = @([ProxyUiSmoke]::Children($window, 'Edit') | Where-Object { [ProxyUiSmoke]::IsWindowVisible($_) })
    if ($edits.Count -ne 4) { throw "Expected 4 settings EDIT controls on the settings page, got $($edits.Count)" }
    [void](Usage)
    $stats = Button 2020
    if (([ProxyUiSmoke]::GetWindowLongW($stats, -16) -band 0x804) -ne 0x804) { throw 'Usage EDIT must be read-only and multiline' }
    Wait-Until 'initial usage snapshot preserves unknown' { (Usage).Contains('输入 未知 / 输出 未知 / 总 未知') }
    # Stats must remain selectable even while configuration controls are disabled.
    if (-not [ProxyUiSmoke]::IsWindowEnabled($stats)) { throw 'Usage details are disabled' }
    Select-Page 3
    if (([ProxyUiSmoke]::GetWindowLongW($edits[3], -16) -band 0x20) -eq 0) { throw 'Upstream key EDIT is not a password control' }
    [ProxyUiSmoke]::SetText($edits[0], [uint32]$child.Id, "http://127.0.0.1:$($mock.Port)/v1")
    [ProxyUiSmoke]::SetText($edits[1], [uint32]$child.Id, '127.0.0.1')
    [ProxyUiSmoke]::SetText($edits[2], [uint32]$child.Id, [string]$port)
    # Synthetic local-only secret verifies real DPAPI, never a real provider key.
    $testSecret = 'local-smoke-secret-' + [Guid]::NewGuid().ToString('N')
    [ProxyUiSmoke]::SetText($edits[3], [uint32]$child.Id, $testSecret)
    # DPAPI remains the default; off mode never injects this secret upstream.
    Click 2001
    Wait-Until 'UI atomic save completed' {
        (Test-Path -LiteralPath $configPath) -and [ProxyUiSmoke]::IsWindowEnabled((Button 2001)) -and (Status).Contains('配置已保存并应用')
    }
    $saved = [IO.File]::ReadAllText($configPath)
    if ($saved.Contains($testSecret) -or -not $saved.Contains('[protected_secrets]')) { throw 'DPAPI did not keep the secret out of config.toml' }
    # Exercise explicit autostart confirmation cancellation only. Never answer Yes,
    # nor send Enable/Disable directly: no actual Startup mutations in any smoke.
    Select-Page 3
    [ProxyUiSmoke]::Send((Button 2039), [uint32]$child.Id, 0x14E, 4)
    [ProxyUiSmoke]::Send($window, [uint32]$child.Id, 0x111, (2039 -bor (1 -shl 16)))
    [ProxyUiSmoke]::Post((Button 2031), [uint32]$child.Id, 0xF5, 0)
    $script:startupDialog = [IntPtr]::Zero
    Wait-Until 'explicit login startup confirmation' { $script:startupDialog = [ProxyUiSmoke]::Dialog([uint32]$child.Id); $script:startupDialog -ne [IntPtr]::Zero }
    [ProxyUiSmoke]::Post([ProxyUiSmoke]::GetDlgItem($script:startupDialog, 7), [uint32]$child.Id, 0xF5, 0)
    Wait-Until 'cancel startup confirmation without writing' { [ProxyUiSmoke]::Dialog([uint32]$child.Id) -eq [IntPtr]::Zero }
    if ([IO.File]::ReadAllText($configPath) -cne $saved) { throw 'Cancelled startup confirmation rewrote config' }
    [ProxyUiSmoke]::Send((Button 2039), [uint32]$child.Id, 0x14E, 0)
    [ProxyUiSmoke]::Send($window, [uint32]$child.Id, 0x111, (2039 -bor (1 -shl 16)))
    if ($LogsOnly) {
        Invoke-LogsSmoke
        Exit-TestGui
        Write-Output 'PASS: tabs and real log controller UI smoke completed.'
        return
    }
    if (-not $saved.Contains("port = $port") -or -not $saved.Contains('auto_start = true') -or -not $saved.Contains('secrets = "dpapi"')) { throw 'Saved settings do not match the GUI input/defaults' }
    if ([ProxyUiSmoke]::PortClosed($port)) { throw 'Save and apply did not start the saved configuration' }
    Click 2003
    Wait-Until 'stop before testing saved-only start' { [ProxyUiSmoke]::IsWindowEnabled((Button 2002)) -and [ProxyUiSmoke]::PortClosed($port) }
    $savedStatus = Status
    [ProxyUiSmoke]::SetText($edits[0], [uint32]$child.Id, 'http://127.0.0.1:1/unsaved')
    [void](Usage)
    [ProxyUiSmoke]::Send($stats, [uint32]$child.Id, 0xB1, 5)
    $selection = [ProxyUiSmoke]::Selection($stats, [uint32]$child.Id)
    Select-Page 2
    if (-not [ProxyUiSmoke]::IsWindowVisible((Button 2200)) -or [ProxyUiSmoke]::IsWindowVisible($stats) -or [ProxyUiSmoke]::IsWindowVisible($edits[0])) { throw 'Log page leaked controls from another page' }
    if (([ProxyUiSmoke]::GetWindowLongW((Button 2200), -16) -band 0x804) -ne 0x804) { throw 'Log EDIT must be read-only and multiline' }
    Wait-Until 'bridged log actions available' { [ProxyUiSmoke]::IsWindowEnabled((Button 2205)) -and [ProxyUiSmoke]::IsWindowEnabled((Button 2219)) }
    Select-Page 0
    Select-Page 1
    if ([ProxyUiSmoke]::Selection($stats, [uint32]$child.Id) -ne $selection) { throw 'Tab switching lost usage selection' }
    Select-Page 3
    Start-Sleep -Milliseconds 2200
    if ([ProxyUiSmoke]::ReadText($edits[0], [uint32]$child.Id) -ne 'http://127.0.0.1:1/unsaved' -or (Status) -cne $savedStatus) { throw 'Usage refresh overwrote unsaved input or operation result' }
    # Keep the invalid draft: Start must use the saved upstream, not this input.
    Write-Output 'PASS: Save and apply starts saved config; usage ticks preserve draft; Start is saved-only.'
    Click 2002
    Wait-Until 'manual start reports running and Stop enabled' { (Status).StartsWith('代理状态：运行中') -and [ProxyUiSmoke]::IsWindowEnabled((Button 2003)) }
    [ProxyUiSmoke]::Request($port, $marker)
    if ([ProxyUiSmoke]::ReadText($edits[0], [uint32]$child.Id) -ne 'http://127.0.0.1:1/unsaved') { throw 'Saved-only Start overwrote the draft' }
    Wait-Until 'mock received forwarded path/query' { $mock.Requests -eq 1 }
    [ProxyUiSmoke]::RequestBody($port, '/usage-json', [ProxyUiSmoke]::UsageJson)
    [ProxyUiSmoke]::RequestBody($port, '/usage-sse', [ProxyUiSmoke]::UsageSse)
    Wait-Until 'JSON and SSE usage accumulated in GUI' {
        $usage = Usage
        $usage.Contains('[全部] 报告 3；完整 2 / 部分 0 / 未知 1') -and
        $usage.Contains('输入 20 / 输出 10 / 总 30；缓存命中 6 / 缓存写入 未知 / 缓存读取 未知；推理 4') -and
        $usage.Contains('OpenAI Chat / local-usage-model')
    }
    if (-not [ProxyUiSmoke]::IsWindowEnabled((Button 2020))) { throw 'Usage disabled while proxy running' }
    # Running configuration editing and SaveAndApply must remain available.
    if (-not [ProxyUiSmoke]::IsWindowEnabled((Button 2001))) { throw 'SaveAndApply is disabled while running' }
    [ProxyUiSmoke]::SetText($edits[0], [uint32]$child.Id, "http://127.0.0.1:$($mock.Port)/v1")
    Click 2001
    Wait-Until 'running SaveAndApply restarts successfully' { (Status).Contains('配置已保存并应用') -and [ProxyUiSmoke]::IsWindowEnabled((Button 2003)) }
    $saved = [IO.File]::ReadAllText($configPath)
    Write-Output 'PASS: saved-only Start, running SaveAndApply and JSON/SSE usage verified; no Startup mutation commands are exercised.'
    Click 2003
    Wait-Until 'GUI reports stopped and listener is closed' { (Status).StartsWith('代理状态：已停止') -and [ProxyUiSmoke]::IsWindowEnabled((Button 2002)) -and [ProxyUiSmoke]::PortClosed($port) }
    $stoppedStatus = Status
    Start-Sleep -Milliseconds 1200
    if ((Status) -cne $stoppedStatus -or -not (Usage).Contains('[全部] 报告 3')) { throw 'Stop result or session usage was lost' }
    Click 2002
    Wait-Until 'same-session proxy restart' { (Status).StartsWith('代理状态：运行中') -and [ProxyUiSmoke]::IsWindowEnabled((Button 2003)) }
    [ProxyUiSmoke]::RequestBody($port, '/usage-json', [ProxyUiSmoke]::UsageJson)
    Wait-Until 'usage survives proxy restart' { (Usage).Contains('[全部] 报告 4；完整 3 / 部分 0 / 未知 1') -and (Usage).Contains('输入 30 / 输出 15 / 总 45') }
    Click 2003
    Wait-Until 'second stop' { [ProxyUiSmoke]::IsWindowEnabled((Button 2002)) -and [ProxyUiSmoke]::PortClosed($port) }
    Exit-TestGui
    Write-Output 'PASS: Stop/start retains session usage; synthetic tray Exit completed normally.'
    Start-TestGui
    Wait-Until 'existing configuration auto-load/auto-start' { (Status).StartsWith('代理状态：运行中') -and [ProxyUiSmoke]::IsWindowEnabled((Button 2003)) }
    Select-Page 3
    $edits = @([ProxyUiSmoke]::Children($window, 'Edit') | Where-Object { [ProxyUiSmoke]::IsWindowVisible($_) })
    if ([ProxyUiSmoke]::ReadText($edits[0], [uint32]$child.Id) -ne "http://127.0.0.1:$($mock.Port)/v1") { throw 'Existing URL was not populated into the GUI' }
    [ProxyUiSmoke]::Request($port, $marker)
    Wait-Until 'new session forwarded request reached mock' { $mock.Requests -eq 5 }
    Wait-Until 'new process resets non-persistent usage' { (Usage).Contains('[全部] 报告 1；完整 0 / 部分 0 / 未知 1') }
    if ([IO.File]::ReadAllText($configPath) -cne $saved) { throw 'Load/start unexpectedly rewrote the configuration' }
    # Fail a running SaveAndApply on a deliberately occupied loopback port.
    # Rollback is never implicit: assert stopped, then explicitly confirm recovery.
    $occupied = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $occupied.Start()
    try {
        $conflictPort = ([Net.IPEndPoint]$occupied.LocalEndpoint).Port
        [ProxyUiSmoke]::SetText($edits[2], [uint32]$child.Id, [string]$conflictPort)
        Click 2001
        Wait-Until 'failed apply reports stopped with rollback guidance' { (Status).StartsWith('代理状态：已停止') -and (Status).Contains('Rollback') -and [ProxyUiSmoke]::IsWindowEnabled((Button 2001)) }
        Select-Page 3
        [ProxyUiSmoke]::Send((Button 2039), [uint32]$child.Id, 0x14E, 4)
        [ProxyUiSmoke]::Send($window, [uint32]$child.Id, 0x111, (2039 -bor (1 -shl 16)))
        [ProxyUiSmoke]::Post((Button 2033), [uint32]$child.Id, 0xF5, 0)
        $script:rollbackDialog = [IntPtr]::Zero
        Wait-Until 'explicit rollback confirmation' { $script:rollbackDialog = [ProxyUiSmoke]::Dialog([uint32]$child.Id); $script:rollbackDialog -ne [IntPtr]::Zero }
        [ProxyUiSmoke]::Post([ProxyUiSmoke]::GetDlgItem($script:rollbackDialog, 6), [uint32]$child.Id, 0xF5, 0)
        Wait-Until 'confirmed rollback resumes previous saved configuration' { (Status).StartsWith('代理状态：运行中') -and [IO.File]::ReadAllText($configPath).Contains("port = $port") }
        [ProxyUiSmoke]::Request($port, $marker)
    } finally { $occupied.Stop() }
    Write-Output 'PASS: real DPAPI secret persistence; cancelled startup confirmation; failed running apply and explicit rollback to saved config.'
    # Exit while running also exercises worker-owned proxy shutdown.
    Exit-TestGui
    if (-not [ProxyUiSmoke]::PortClosed($port)) { throw 'Listener still open after Exit' }
    Write-Output 'PASS: existing config auto-loaded/auto-started, forwarded another request, then Exit released the listener without rewriting config.'
    Write-Output 'LIMITS: synthetic control clicks/Exit command; no physical tray clicks, TLS upstream, plaintext consent, DPI or Explorer restart validation.'
} finally {
    try {
        if ($null -ne $child) {
            try {
                if (-not $child.HasExited) {
                    Write-Output "CLEANUP: terminating only test-created PID $($child.Id)."
                    $child.Kill()
                    if (-not $child.WaitForExit(3000)) { throw 'Test child cleanup timed out' }
                }
            } finally { $child.Dispose() }
        }
    } finally {
        try { if ($null -ne $mock) { $mock.Dispose() } }
        finally { if ($null -ne $temp -and (Test-Path -LiteralPath $temp)) { Remove-Item -LiteralPath $temp -Recurse -Force } }
    }
}
