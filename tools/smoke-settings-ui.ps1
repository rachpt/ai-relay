# Own GUI/temp directory and loopback mocks only. Never accepts a Startup change.
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
using System.Net.Sockets;
using System.Text;
public static class SettingsRequest {
    static TcpClient Connect(int port) {
        var client=new TcpClient();
        try { if(!client.ConnectAsync(IPAddress.Loopback,port).Wait(3000)) throw new Exception("Connect timeout"); }
        catch { client.Dispose(); throw; }
        client.ReceiveTimeout=3000; client.SendTimeout=3000; return client;
    }
    public static string Send(int port,string target,string token,string extra) {
        using(var client=Connect(port)) using(var stream=client.GetStream()) {
            var bytes=Encoding.ASCII.GetBytes("GET "+target+" HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n"+(token==null?"":"Proxy-Authorization: Bearer "+token+"\r\n")+extra+"\r\n");
            stream.Write(bytes,0,bytes.Length);
            using(var reader=new StreamReader(stream)) return reader.ReadToEnd();
        }
    }
    public static string Preflight(int port,string origin) {
        using(var client=Connect(port)) using(var stream=client.GetStream()) {
            var bytes=Encoding.ASCII.GetBytes("OPTIONS /smoke HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nOrigin: "+origin+"\r\nAccess-Control-Request-Method: GET\r\nAccess-Control-Request-Headers: proxy-authorization\r\n\r\n");
            stream.Write(bytes,0,bytes.Length);
            using(var reader=new StreamReader(stream)) return reader.ReadToEnd();
        }
    }
    public static void Tunnel(int proxyPort,int targetPort,string token) {
        using(var listener=new TcpListenerWrapper()) {
            // The GUI-allowed port is the actual loopback target supplied by the caller.
            if(listener.Port!=targetPort) throw new Exception("Use TunnelToListener instead");
        }
    }
    public static void TunnelToListener(int proxyPort,TcpListener listener,string token) {
        int port=((IPEndPoint)listener.LocalEndpoint).Port;
        var accepted=listener.AcceptTcpClientAsync();
        using(var client=Connect(proxyPort)) using(var stream=client.GetStream()) {
            var bytes=Encoding.ASCII.GetBytes("CONNECT 127.0.0.1:"+port+" HTTP/1.1\r\nHost: 127.0.0.1:"+port+"\r\nProxy-Authorization: Bearer "+token+"\r\n\r\n");
            stream.Write(bytes,0,bytes.Length);
            var header=new StringBuilder();
            while(!header.ToString().EndsWith("\r\n\r\n")) {
                int b=stream.ReadByte(); if(b<0 || header.Length>16384) throw new Exception("Incomplete CONNECT response"); header.Append((char)b);
            }
            if(!header.ToString().StartsWith("HTTP/1.1 200")) throw new Exception("CONNECT not established: "+header);
            if(!accepted.Wait(3000)) throw new Exception("Tunnel target accept timeout");
            using(var target=accepted.Result) using(var upstream=target.GetStream()) {
                target.ReceiveTimeout=3000; target.SendTimeout=3000;
                stream.WriteByte(0x41);
                if(upstream.ReadByte()!=0x41) throw new Exception("Tunnel upload mismatch or credential injection");
                upstream.WriteByte(0x42);
                if(stream.ReadByte()!=0x42) throw new Exception("Tunnel download mismatch");
            }
        }
    }
    sealed class TcpListenerWrapper : IDisposable {
        readonly TcpListener listener=new TcpListener(IPAddress.Loopback,0);
        public int Port { get { return ((IPEndPoint)listener.LocalEndpoint).Port; } }
        public TcpListenerWrapper() { listener.Start(); }
        public void Dispose() { listener.Stop(); }
    }
}
'@
$temp=Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-settings-ui-'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temp)
$child=$null; $mock=$null; $target=$null; $window=[IntPtr]::Zero
$overall=[Diagnostics.Stopwatch]::StartNew()
function Control([int]$Id) { [ProxyUiSmoke]::GetDlgItem($window,$Id) }
function Text([int]$Id) { [ProxyUiSmoke]::ReadText((Control $Id),[uint32]$child.Id) }
function Status {
    foreach($h in [ProxyUiSmoke]::Children($window,'Static')) {
        $text=[ProxyUiSmoke]::Title($h)
        if($text.StartsWith('代理状态：')) { return $text }
    }
    return ''
}
function Wait([string]$Name,[scriptblock]$Test) {
    $timer=[Diagnostics.Stopwatch]::StartNew()
    while($timer.Elapsed.TotalSeconds -lt $StepTimeoutSeconds -and $overall.Elapsed.TotalSeconds -lt 180) {
        if($child.HasExited) { throw "Child exited: $Name" }
        if(& $Test) { return }
        Start-Sleep -Milliseconds 60
    }
    throw ("Timeout: $Name; status="+(Status))
}
function Click([int]$Id) {
    if(-not [ProxyUiSmoke]::IsWindowEnabled((Control $Id))) { throw "Disabled control $Id" }
    [ProxyUiSmoke]::Post((Control $Id),[uint32]$child.Id,0xF5,0)
}
function Category([uint32]$Index) {
    [ProxyUiSmoke]::SelectTab($window,[uint32]$child.Id,3)
    [ProxyUiSmoke]::Send((Control 2039),[uint32]$child.Id,0x14E,$Index)
    [ProxyUiSmoke]::Send($window,[uint32]$child.Id,0x111,(2039 -bor (1 -shl 16)))
}
function Set-Field([int]$Id,[string]$Value) { [ProxyUiSmoke]::SetText((Control $Id),[uint32]$child.Id,$Value) }
function Check([int]$Id,[bool]$Value) { [ProxyUiSmoke]::Send((Control $Id),[uint32]$child.Id,0xF1,[uint32]$Value) }
function Answer([int]$Id) {
    $script:dialog=[IntPtr]::Zero
    Wait 'risk confirmation' { $script:dialog=[ProxyUiSmoke]::Dialog([uint32]$child.Id); $dialog -ne [IntPtr]::Zero }
    $answered=$dialog
    [ProxyUiSmoke]::Post([ProxyUiSmoke]::GetDlgItem($dialog,$Id),[uint32]$child.Id,0xF5,0)
    Wait 'confirmation closed' { [ProxyUiSmoke]::Dialog([uint32]$child.Id) -ne $answered }
}
function Save {
    Click 2001
    Answer 6 # Protocol risk only; never Startup or non-loopback authorization.
    Wait 'saved and applied' { (Status).Contains('配置已保存并应用') -and [ProxyUiSmoke]::IsWindowEnabled((Control 2001)) }
}
function Assert-Status([string]$Response,[int]$Code) {
    if(-not $Response.StartsWith("HTTP/1.1 $Code")) { throw "Unexpected response: $Response" }
}
try {
    $mock=[ProxyUiMock]::new('local-settings-marker')
    $target=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0); $target.Start()
    $targetPort=([Net.IPEndPoint]$target.LocalEndpoint).Port
    $port=[ProxyUiSmoke]::FreePort()
    $config=Join-Path $temp 'config.toml'
    [IO.File]::WriteAllText($config,"schema_version = 1`n[upstream]`nurl = 'http://127.0.0.1:$($mock.Port)/v1'`n[listen]`nport = $port`n[proxy]`nauto_start = false`n")
    $child=Start-Process -FilePath $exe -ArgumentList @('--config-dir',('"'+$temp+'"')) -PassThru
    Wait 'loaded GUI' { $script:window=[ProxyUiSmoke]::Find([uint32]$child.Id); $window -ne [IntPtr]::Zero -and (Status).Contains('配置已加载') -and [ProxyUiSmoke]::IsWindowEnabled((Control 2001)) }
    $original=[IO.File]::ReadAllText($config)
    Category 1
    if(([ProxyUiSmoke]::GetWindowLongW((Control 2042),-16) -band 0x20) -eq 0) { throw 'Client token not masked' }
    $token='synthetic-proxy-client-'+[Guid]::NewGuid().ToString('N')
    Set-Field 2040 '["127.0.0.1", "::1", "192.0.2.0/24"]'
    Check 2041 $true
    Set-Field 2042 $token
    Category 2
    Check 2043 $true; Check 2044 $true; Check 2045 $true
    Set-Field 2046 "[$targetPort]"
    Check 2047 $true; Set-Field 2048 '["https://client.invalid"]'; Check 2049 $true
    # Native page switches do not save or lose another category's secret draft.
    Category 1
    # Windows may block cross-process WM_GETTEXT for password controls. Verify
    # secret retention through the authenticated request after saving, not by reading it.
    if([IO.File]::ReadAllText($config) -cne $original) { throw 'Category switch implicitly saved' }
    Save
    $saved=[IO.File]::ReadAllText($config)
    foreach($setting in @('client_token_enabled = true','forward_enabled = true','connect_enabled = true','allow_credentials = true','https://client.invalid','192.0.2.0/24')) {
        if(-not $saved.Contains($setting)) { throw "Setting not persisted: $setting" }
    }
    if($saved.Contains($token) -or -not $saved.Contains('[protected_secrets]')) { throw 'Client token DPAPI persistence failed' }
    Assert-Status ([SettingsRequest]::Send($port,'/smoke?probe=1',$null,'')) 407
    Assert-Status ([SettingsRequest]::Send($port,'/smoke?probe=1','wrong-synthetic-token','')) 407
    $response=[SettingsRequest]::Send($port,'/smoke?probe=1',$token,"Origin: https://client.invalid`r`n")
    Assert-Status $response 200
    if(-not $response.Contains('local-settings-marker') -or -not $response.ToLowerInvariant().Contains('access-control-allow-origin: https://client.invalid') -or -not $response.ToLowerInvariant().Contains('access-control-allow-credentials: true')) { throw 'CORS response policy not applied' }
    $before=$mock.Requests
    Assert-Status ([SettingsRequest]::Preflight($port,'https://client.invalid')) 204
    Assert-Status ([SettingsRequest]::Preflight($port,'https://denied.invalid')) 403
    if($mock.Requests -ne $before) { throw 'Preflight reached upstream' }
    # A deliberately unreachable loopback target proves rewrite-to-configured-upstream.
    Assert-Status ([SettingsRequest]::Send($port,'http://127.0.0.1:1/smoke?probe=1',$token,'')) 200
    [SettingsRequest]::TunnelToListener($port,$target,$token)
    if($mock.ProxyCredentialsReceived -ne 0) { throw 'Proxy token leaked upstream' }
    Category 2
    Check 2044 $false
    Save
    Assert-Status ([SettingsRequest]::Send($port,"http://127.0.0.1:$($mock.Port)/v1/smoke?probe=1",$token,'')) 200
    $saved=[IO.File]::ReadAllText($config)
    # Invalid CIDR must fail locally with a field-only error, preserving config/listener.
    Category 1
    Set-Field 2040 '["invalid-synthetic-address"]'
    Click 2001
    $script:dialog=[IntPtr]::Zero
    Wait 'invalid whitelist error' { $script:dialog=[ProxyUiSmoke]::Dialog([uint32]$child.Id); $dialog -ne [IntPtr]::Zero }
    $messages=(@([ProxyUiSmoke]::Children($dialog,'Static') | ForEach-Object { [ProxyUiSmoke]::Title($_) }) -join ' ')
    if(-not $messages.Contains('access.whitelist') -or $messages.Contains('invalid-synthetic-address')) { throw 'Missing field error or input echoed' }
    # Error dialog uses an informational/no-default confirmation style; dismiss
    # without assuming the OK button exists.
    [ProxyUiSmoke]::Post($dialog,[uint32]$child.Id,0x10,0)
    Wait 'validation dialog closed' { [ProxyUiSmoke]::Dialog([uint32]$child.Id) -eq [IntPtr]::Zero }
    if([IO.File]::ReadAllText($config) -cne $saved) { throw 'Invalid draft rewrote configuration' }
    Category 4; Click 2037
    Category 1
    if((Text 2040).Contains('invalid')) { throw 'Discard did not restore verified config' }
    Assert-Status ([SettingsRequest]::Send($port,'/smoke?probe=1',$token,'')) 200
    Category 5; Click 2062
    Wait 'real log settings shortcut' { [ProxyUiSmoke]::IsWindowVisible((Control 2219)) -and -not [ProxyUiSmoke]::IsWindowVisible((Control 2001)) }
    [ProxyUiSmoke]::Post($window,[uint32]$child.Id,0x111,1002)
    if(-not $child.WaitForExit(38000) -or $child.ExitCode -ne 0) { throw 'Graceful exit failed' }
    $child.Dispose(); $child=$null
    Write-Output 'PASS: category/draft retention, whitelist validation/discard, independent masked client token/DPAPI/407, forward rewrite and direct mode, real opaque CONNECT, CORS/preflight isolation, proxy-token stripping and log settings shortcut.'
    Write-Output 'LIMITS: loopback/synthetic input only; no non-loopback authorization, real Startup mutation, TLS handshake, physical keyboard or DPI acceptance.'
} catch {
    Write-Output $_.ScriptStackTrace
    throw
} finally {
    if($null -ne $child) { if(-not $child.HasExited) { $child.Kill(); [void]$child.WaitForExit(10000) }; $child.Dispose() }
    if($null -ne $target) { $target.Stop() }
    if($null -ne $mock) { $mock.Dispose() }
    if([IO.Directory]::Exists($temp)) { Remove-Item -LiteralPath $temp -Recurse -Force }
}
