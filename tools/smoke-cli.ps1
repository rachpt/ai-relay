# Windows-only, local mock and temporary config; never touches real Startup.
param([string]$ExePath = (Join-Path $PSScriptRoot '..\target\debug\ai-relay.exe'))
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path -LiteralPath $ExePath).Path
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$session = [Diagnostics.Process]::GetCurrentProcess().SessionId
$mutexName = "Local\AIRelay.$sid.$session.v1"
try { $existing = [Threading.Mutex]::OpenExisting($mutexName) } catch [Threading.WaitHandleCannotBeOpenedException] { $existing = $null }
if ($existing) { $existing.Dispose(); throw 'Existing instance: refusing to affect user state.' }
$dir = Join-Path ([IO.Path]::GetTempPath()) ('ai-relay CLI ' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $dir | Out-Null
Add-Type @'
using System;
using System.Diagnostics;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading.Tasks;
public static class CliSmoke {
    // ProcessStartInfo.ArgumentList is unavailable in Windows PowerShell 5.1.
    // This is CRT argument quoting, not a shell command or cmd.exe invocation.
    public static string Quote(string s) {
        var b = new StringBuilder("\""); int slashes = 0;
        foreach(char c in s) {
            if(c == '\\') { slashes++; continue; }
            b.Append('\\', c == '"' ? slashes * 2 + 1 : slashes); b.Append(c); slashes = 0;
        }
        b.Append('\\', slashes * 2); return b.Append('"').ToString();
    }
    public static string[] Run(string exe, string[] args, int expected) {
        var info = new ProcessStartInfo(exe); info.UseShellExecute = false;
        info.CreateNoWindow = true; info.RedirectStandardOutput = true; info.RedirectStandardError = true;
        info.StandardOutputEncoding = Encoding.UTF8; info.StandardErrorEncoding = Encoding.UTF8;
        var command = new StringBuilder();
        foreach(var arg in args) { if(command.Length > 0) command.Append(' '); command.Append(Quote(arg)); }
        info.Arguments = command.ToString();
        using(var p = Process.Start(info)) {
            var stdout = p.StandardOutput.ReadToEndAsync(); var stderr = p.StandardError.ReadToEndAsync();
            if(!p.WaitForExit(48000)) { p.Kill(); throw new Exception("CLI exceeded bounded wait"); }
            if(!Task.WaitAll(new Task[] {stdout, stderr}, 3000)) throw new Exception("Background process inherited CLI output pipe");
            if(p.ExitCode != expected) throw new Exception("Expected exit " + expected + ", got " + p.ExitCode + ": " + stdout.Result + stderr.Result);
            return new string[] { stdout.Result, stderr.Result };
        }
    }
    public static int FreePort() { var l = new TcpListener(IPAddress.Loopback, 0); l.Start(); int port = ((IPEndPoint)l.LocalEndpoint).Port; l.Stop(); return port; }
    public static void Request(int port) {
        var r = (HttpWebRequest)WebRequest.Create("http://127.0.0.1:" + port + "/cli-smoke");
        r.Proxy = null; r.Timeout = 3000; r.ReadWriteTimeout = 3000;
        using(var response = r.GetResponse()) using(var reader = new StreamReader(response.GetResponseStream())) {
            if(reader.ReadToEnd() != "local-cli-mock") throw new Exception("Unexpected mock response");
        }
    }
    public delegate bool Callback(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] static extern bool EnumWindows(Callback cb, IntPtr p);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder text, int length);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool GetNamedPipeServerProcessId(IntPtr pipe, out uint pid);
    public static bool Visible(uint process) {
        bool visible = false;
        EnumWindows(delegate(IntPtr h, IntPtr unused) {
            uint owner; GetWindowThreadProcessId(h, out owner);
            var name = new StringBuilder(256); GetClassName(h, name, 256);
            if(owner == process && name.ToString() == "AIRelay.Window.v1" && IsWindowVisible(h)) visible = true;
            return true;
        }, IntPtr.Zero);
        return visible;
    }
}
public sealed class CliMock : IDisposable {
    readonly TcpListener listener; readonly Task worker; volatile bool stopping;
    public int Port { get; private set; }
    public CliMock() {
        listener = new TcpListener(IPAddress.Loopback, 0); listener.Start(); Port = ((IPEndPoint)listener.LocalEndpoint).Port;
        worker = Task.Run(async delegate {
            try { while(!stopping) {
                using(var client = await listener.AcceptTcpClientAsync()) {
                    client.ReceiveTimeout = 3000; client.SendTimeout = 3000;
                    using(var stream = client.GetStream()) using(var reader = new StreamReader(stream, Encoding.ASCII, false, 1024, true)) {
                        string line; do { line = reader.ReadLine(); } while(line != null && line.Length != 0);
                        var bytes = Encoding.ASCII.GetBytes("HTTP/1.1 200 OK\r\nContent-Length: 14\r\nConnection: close\r\n\r\nlocal-cli-mock");
                        stream.Write(bytes, 0, bytes.Length);
                    }
                }
            } } catch(Exception) { if(!stopping) throw; }
        });
    }
    public void Dispose() { stopping = true; listener.Stop(); if(!worker.Wait(5000)) throw new Exception("Mock did not stop"); }
}
'@
$mock = New-Object CliMock
$port = [CliSmoke]::FreePort()
$config = @"
schema_version = 1
[upstream]
url = "http://127.0.0.1:$($mock.Port)"
[listen]
address = "127.0.0.1"
port = $port
[proxy]
auto_start = false
"@
[IO.File]::WriteAllText((Join-Path $dir 'config.toml'), $config, (New-Object Text.UTF8Encoding($false)))
$hosts = New-Object 'System.Collections.Generic.List[System.Diagnostics.Process]'
function Invoke-Cli([string[]]$Arguments, [int]$Expected) {
    $result = [CliSmoke]::Run($exe, $Arguments, $Expected)
    if (($result[0] + $result[1]).Length -eq 0) { throw 'Expected redirected CLI output' }
    Write-Host (($Arguments -join ' ') + ' => ' + $Expected + ' ' + ($result -join '').Trim())
}
function Capture-Host([bool]$AllowVisible = $false) {
    $pipe = New-Object IO.Pipes.NamedPipeClientStream('.', "AIRelay.$sid.$session.v1", [IO.Pipes.PipeDirection]::InOut)
    try {
        $pipe.Connect(3000); [uint32]$server = 0
        if (![CliSmoke]::GetNamedPipeServerProcessId($pipe.SafePipeHandle.DangerousGetHandle(), [ref]$server)) { throw 'Cannot identify smoke host' }
        $hostProcess = [Diagnostics.Process]::GetProcessById($server)
        if ($hostProcess.MainModule.FileName -ne $exe) { throw 'Unexpected IPC server executable' }
        $identity = Get-CimInstance Win32_Process -Filter "ProcessId = $server"
        if (!$identity.CommandLine.Contains([CliSmoke]::Quote($dir))) { $hostProcess.Dispose(); throw 'IPC server is not using this smoke temporary directory' }
        $hosts.Add($hostProcess)
        if (!$AllowVisible -and [CliSmoke]::Visible($server)) { throw 'Background startup showed main window' }
    } finally { $pipe.Dispose() }
}
function Wait-Hosts {
    foreach ($hostProcess in $hosts) {
        if (!$hostProcess.WaitForExit(40000)) { throw 'Graceful host exit exceeded bound' }
    }
}
try {
    Invoke-Cli @('--help') 0
    Invoke-Cli @('--version') 0
    Invoke-Cli @('status') 1
    Invoke-Cli @('stop') 0
    Invoke-Cli @('quit') 0
    Invoke-Cli @('--invalid') 2
    Invoke-Cli @('--background', '--initial-action', 'quit') 2
    $bad = Join-Path $dir 'bad config'; New-Item -ItemType Directory $bad | Out-Null
    [IO.File]::WriteAllText((Join-Path $bad 'config.toml'), 'invalid toml = [')
    Invoke-Cli @('start', '--config-dir', $bad) 2
    Invoke-Cli @('start', '--config-dir', $dir) 0
    Capture-Host
    Invoke-Cli @('status', '--config-dir', $dir) 0
    [CliSmoke]::Request($port)
    Invoke-Cli @('restart', '--config-dir', $dir) 0
    [CliSmoke]::Request($port)
    Invoke-Cli @('stop', '--config-dir', $dir) 0
    Invoke-Cli @('stop', '--config-dir', $dir) 0
    Invoke-Cli @('status', '--config-dir', $dir) 1
    Invoke-Cli @('quit', '--config-dir', $dir) 0
    Wait-Hosts
    Invoke-Cli @('quit') 0
    Invoke-Cli @('restart', '--config-dir', $dir) 0
    Capture-Host
    [CliSmoke]::Request($port)
    Invoke-Cli @('quit', '--config-dir', $dir) 0
    Wait-Hosts
    # A bind failure is an operation failure (3), not a false successful launch.
    $busy = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, $port)
    $busy.Start()
    try {
        Invoke-Cli @('start', '--config-dir', $dir) 3
        Capture-Host $true
        Invoke-Cli @('quit', '--config-dir', $dir) 0
        Wait-Hosts
    } finally { $busy.Stop() }
    Invoke-Cli @('status') 1
    Write-Host 'PASS: CLI 0/1/2/3, redirected output, hidden background start/restart, auto_start=false, local mock, stop/quit'
} catch {
    Write-Host ('SMOKE FAILURE: ' + $_.Exception.ToString())
    throw
} finally {
    # Recover a newly launched host even if the launching CLI assertion failed.
    try {
        $remaining = $null
        try { $remaining = [Threading.Mutex]::OpenExisting($mutexName) } catch [Threading.WaitHandleCannotBeOpenedException] { }
        if ($remaining) {
            $remaining.Dispose()
            if ($hosts.Count -eq 0 -or $hosts[$hosts.Count - 1].HasExited) { Capture-Host $true }
        }
    } catch { Write-Host ('Cleanup capture: ' + $_.Exception.Message) }
    # No PID-file kills: only the captured exact smoke process objects are stopped.
    foreach ($hostProcess in $hosts) {
        if (!$hostProcess.HasExited) { $hostProcess.Kill(); $hostProcess.WaitForExit() }
        $hostProcess.Dispose()
    }
    $mock.Dispose()
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction Continue
}
