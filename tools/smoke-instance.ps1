param([Parameter(Mandatory=$true)][string]$Exe)
$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$session = [Diagnostics.Process]::GetCurrentProcess().SessionId
$name = "Local\AIRelay.$sid.$session.v1"
try { $existing = [Threading.Mutex]::OpenExisting($name) } catch [Threading.WaitHandleCannotBeOpenedException] { $existing = $null }
if ($existing) { $existing.Dispose(); throw 'Existing application instance: refusing to affect user state.' }
$dir = Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-instance-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory $dir | Out-Null
$hostProcess = $null
function Invoke-Control([string[]]$Arguments, [int]$Expected) {
    $out = Join-Path $dir 'stdout.txt'; $err = Join-Path $dir 'stderr.txt'
    $info = @{FilePath=$Exe; PassThru=$true; RedirectStandardOutput=$out; RedirectStandardError=$err}
    if ($Arguments.Count) { $info.ArgumentList = $Arguments }
    $p = Start-Process @info
    $handle = $p.Handle
    if (!$p.WaitForExit(45000)) { $p.Kill(); throw 'CLI exceeded bounded wait' }

    $text = (Get-Content $out -Raw -Encoding UTF8) + (Get-Content $err -Raw -Encoding UTF8)
    if ($p.ExitCode -ne $Expected) { throw "Unexpected exit $($p.ExitCode): $text" }
    Write-Host ($Arguments -join ' ') $text
}
try {
    Invoke-Control @('--help') 0
    Invoke-Control @('--version') 0
    Invoke-Control @('status') 1
    Invoke-Control @('stop') 0
    Invoke-Control @('quit') 0
    Invoke-Control @('start') 1
    Invoke-Control @('restart') 1
    # Pre-existing endpoint must cause fail-closed startup, not a second server.
    $squatter = New-Object IO.Pipes.NamedPipeServerStream("AIRelay.$sid.$session.v1",[IO.Pipes.PipeDirection]::InOut,2)
    try { Invoke-Control @('--background','--config-dir',('"' + $dir + '"')) 1 } finally { $squatter.Dispose() }
    $hostProcess = Start-Process -FilePath $Exe -ArgumentList @('--background','--config-dir',('"' + $dir + '"')) -PassThru
    Start-Sleep -Seconds 2
    if ($hostProcess.HasExited) { throw 'Primary exited prematurely' }
    Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class InstanceWindows {
 public delegate bool Callback(IntPtr h, IntPtr p);
 [DllImport("user32.dll")] public static extern bool EnumWindows(Callback cb, IntPtr p);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
 [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder name, int size);
}
'@
    function Test-Visible {
        $script:visible = $false
        [InstanceWindows]::EnumWindows({ param($h,$unused)
            [uint32]$owner = 0
            [InstanceWindows]::GetWindowThreadProcessId($h,[ref]$owner) | Out-Null
            $class = New-Object Text.StringBuilder 256
            [InstanceWindows]::GetClassName($h,$class,256) | Out-Null
            if ($owner -eq $hostProcess.Id -and $class.ToString() -eq 'AIRelay.Window.v1' -and [InstanceWindows]::IsWindowVisible($h)) { $script:visible = $true }
            return $true
        }, [IntPtr]::Zero) | Out-Null
        return $script:visible
    }
    if (Test-Visible) { throw 'Background launch unexpectedly visible' }
    Invoke-Control @('status') 1
    Invoke-Control @('stop') 0
    Invoke-Control @('show','--config-dir',('"' + (Join-Path $dir 'different') + '"')) 1
    if (Test-Visible) { throw 'Rejected config switch showed window' }
    # Version mismatch is a structured rejection, with no controller execution.
    $versionPipe = New-Object IO.Pipes.NamedPipeClientStream('.',"AIRelay.$sid.$session.v1",[IO.Pipes.PipeDirection]::InOut)
    $versionPipe.Connect(3000)
    $writer = New-Object IO.BinaryWriter($versionPipe)
    $reader = New-Object IO.BinaryReader($versionPipe)
    $payload = [Text.Encoding]::UTF8.GetBytes('{"version":999,"action":"quit","directory":null}')
    $writer.Write([uint32]$payload.Length); $writer.Write($payload); $writer.Flush()
    $length = $reader.ReadUInt32()
    if ($length -gt 4096) { throw 'Unbounded response' }
    $reply = [Text.Encoding]::UTF8.GetString($reader.ReadBytes($length)) | ConvertFrom-Json
    if ($reply.ok -or $reply.version -ne 1) { throw 'Version mismatch not rejected' }
    $writer.Write([byte]1); $writer.Flush()
    $versionPipe.Dispose()
    Start-Sleep -Milliseconds 150
    # A slow peer sends a frame prefix only. Server must time out and recover.
    $pipe = New-Object IO.Pipes.NamedPipeClientStream('.',"AIRelay.$sid.$session.v1",[IO.Pipes.PipeDirection]::InOut)
    $pipe.Connect(3000)
    $pipe.WriteByte(100)
    Start-Sleep -Seconds 3
    $pipe.Dispose()
    Invoke-Control @() 0
    Start-Sleep -Milliseconds 300
    if (!(Test-Visible)) { throw 'Duplicate launch did not show primary window' }
    Invoke-Control @('show','--config-dir',('"' + $dir + '"')) 0
    Invoke-Control @('quit') 0
    if (!$hostProcess.WaitForExit(40000)) { throw 'Graceful quit timed out' }
    Invoke-Control @('status') 1
    Write-Host 'PASS: local instance smoke; no config saved, no autostart changes'
} finally {
    if ($hostProcess -and !$hostProcess.HasExited) { $hostProcess.Kill(); $hostProcess.WaitForExit() }
    Remove-Item -Recurse -Force $dir
}
