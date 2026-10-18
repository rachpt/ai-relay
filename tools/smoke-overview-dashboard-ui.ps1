# Windows PowerShell 5.1 x64 / interactive Windows 10/11 desktop.
# Only our pinned child, private config and loopback mock. Never builds or runs old tests.
[CmdletBinding()]
param(
    [string]$ExePath,
    [switch]$ValidateOnly
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([IntPtr]::Size -ne 8) { throw 'Use a fresh 64-bit powershell.exe -NoProfile process' }
if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $PSScriptRoot '..\target\overview-dashboard-validation\target\debug\ai-relay.exe'
}

# Extract C# DECLARATIONS only. No dot-sourcing/Invoke-Expression/old script invocation.
$parts = @(
    @{ File='smoke-proxy-ui.ps1'; Pattern="(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@" },
    @{ File='smoke-ui-clarity.ps1'; Pattern="(?s)Add-Type -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll -TypeDefinition @'\r?\n(.*?)\r?\n'@" },
    @{ File='smoke-status-bar-ui.ps1'; Pattern='(?s)\$statusDeclaration = @''\r?\n(.*?)\r?\n''@' },
    @{ File='smoke-modern-ui.ps1'; Pattern='(?s)\$declarations.Add\(@''\r?\n(.*?)\r?\n''@\)' },
    @{ File='smoke-window-ui.ps1'; Pattern="(?s)\+ @'\r?\n(.*?)\r?\n'@\)" }
)
foreach ($name in @('ProxyUiSmoke','ProxyUiMock','ClarityNative','StatusBarNative','ModernNative','WindowSmoke','WindowTray','DashboardNative','DashboardMock')) {
    if ($name -as [type]) { throw "Helper $name already loaded; use a fresh powershell.exe -NoProfile" }
}
$declarations = [Collections.Generic.List[string]]::new()
foreach ($part in $parts) {
    $source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot $part.File))
    $match = [regex]::Match($source,$part.Pattern)
    if (-not $match.Success) { throw "Missing declaration in $($part.File); no older test executed" }
    $declarations.Add($match.Groups[1].Value)
}
$declarations.Add(@'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
public static class DashboardNative {
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int w,int height,uint flags);
    [DllImport("user32.dll")] static extern bool InvalidateRect(IntPtr h,IntPtr rect,bool erase);
    public static void Resize(IntPtr h,uint pid,int x,int y,int w,int height) {
        ProxyUiSmoke.CheckOwner(h,pid);
        var previous=StatusBarNative.EnterDpi();
        try {
            if(!SetWindowPos(h,IntPtr.Zero,x,y,w,height,0x4014)) throw new Exception("Async physical resize failed");
        } finally { StatusBarNative.RestoreDpi(previous); }
    }
    public static void Invalidate(IntPtr h,uint pid) {
        ProxyUiSmoke.CheckOwner(h,pid);
        if(!InvalidateRect(h,IntPtr.Zero,false)) throw new Exception("Dashboard invalidation failed");
    }
    public sealed class RegionProof {
        public string Name;
        public int X,Y,Width,Height,Blue,Red,Teal,Ink,Colored;
    }
    // Rough pixel evidence, NOT OCR or proof of individual numerical glyphs.
    public static RegionProof[] Inspect(string path,uint dpi,bool dark) {
        var result=new List<RegionProof>();
        double s=dpi/96.0;
        using(var image=new Bitmap(path)) {
            int gap=(int)Math.Round(12*s), cardHeight=(int)Math.Round(112*s);
            int cardWidth=(image.Width-3*gap)/4;
            for(int i=0;i<4;i++) result.Add(Region(image,"card-"+i,i*(cardWidth+gap),0,i==3?image.Width-i*(cardWidth+gap):cardWidth,cardHeight,dark));
            int panelWidth=(image.Width-gap)/2, top=(int)Math.Round(124*s);
            int panelHeight=image.Height-top-(int)Math.Round(24*s);
            for(int i=0;i<2;i++) {
                int left=i*(panelWidth+gap);
                result.Add(Region(image,"chart-"+i+"-legend",left,top+(int)Math.Round(32*s),panelWidth,(int)Math.Round(22*s),dark));
                result.Add(Region(image,"chart-"+i+"-units",left+(int)Math.Round(6*s),top+(int)Math.Round(76*s),(int)Math.Round(52*s),(int)Math.Round(18*s),dark));
                result.Add(Region(image,"chart-"+i+"-plot",left+(int)Math.Round(62*s),top+(int)Math.Round(108*s),panelWidth-(int)Math.Round(80*s),panelHeight-(int)Math.Round(148*s),dark));
            }
        }
        return result.ToArray();
    }
    static RegionProof Region(Bitmap b,string name,int x,int y,int w,int h,bool dark) {
        if(w<1 || h<1 || x<0 || y<0 || x+w>b.Width || y+h>b.Height) throw new Exception("Physical chart region clipped: "+name);
        var p=new RegionProof { Name=name,X=x,Y=y,Width=w,Height=h };
        // Inspect every pixel so 1px legend dashes are not missed at fractional DPI.
        for(int yy=y;yy<y+h;yy++) for(int xx=x;xx<x+w;xx++) {
            var c=b.GetPixel(xx,yy);
            if(c.B>c.R+25 && c.B>c.G+12) p.Blue++;
            if(c.R>c.G+25 && c.R>c.B+25) p.Red++;
            if(c.G>c.R+18 && c.B>c.R+18 && c.G>=c.B-25) p.Teal++;
            if(Math.Max(c.R,Math.Max(c.G,c.B))-Math.Min(c.R,Math.Min(c.G,c.B))>45) p.Colored++;
            int l=(int)(0.2126*c.R+0.7152*c.G+0.0722*c.B);
            if(dark?l>150:l<125) p.Ink++;
        }
        return p;
    }
}
public sealed class DashboardMock : IDisposable {
    readonly TcpListener listener;
    readonly Task worker;
    volatile bool stopping;
    int successes,errors;
    long uploaded;
    public int Port { get; private set; }
    public int Successes { get { return Volatile.Read(ref successes); } }
    public int Errors { get { return Volatile.Read(ref errors); } }
    public long Uploaded { get { return Interlocked.Read(ref uploaded); } }
    public static readonly string Payload="{\"model\":\"local-dashboard-only\",\"input\":\"本地字节验收\"}";
    public static readonly string Response="{\"model\":\"local-dashboard-only\",\"result\":\"仅回环模拟响应\"}";
    public static int UploadBytes { get { return Encoding.UTF8.GetByteCount(Payload); } }
    public static int DownloadBytes { get { return Encoding.UTF8.GetByteCount(Response); } }
    public DashboardMock() {
        listener=new TcpListener(IPAddress.Loopback,0); listener.Start();
        Port=((IPEndPoint)listener.LocalEndpoint).Port;
        worker=Task.Run(delegate {
            try {
                while(!stopping) using(var client=listener.AcceptTcpClient()) {
                    client.ReceiveTimeout=2500; client.SendTimeout=2500;
                    using(var stream=client.GetStream()) {
                        var header=new List<byte>(); int b;
                        while((b=stream.ReadByte())!=-1) {
                            header.Add((byte)b); int n=header.Count;
                            if(n>16384) throw new Exception("Mock header limit exceeded");
                            if(n>=4 && header[n-4]==13 && header[n-3]==10 && header[n-2]==13 && header[n-1]==10) break;
                        }
                        string text=Encoding.ASCII.GetString(header.ToArray());
                        string[] lines=text.Split(new string[]{"\r\n"},StringSplitOptions.None);
                        int length=0;
                        foreach(string line in lines) {
                            if(line.StartsWith("Content-Length:",StringComparison.OrdinalIgnoreCase)) length=Int32.Parse(line.Substring(15).Trim(),System.Globalization.CultureInfo.InvariantCulture);
                            if(line.StartsWith("Authorization:",StringComparison.OrdinalIgnoreCase) || line.StartsWith("Proxy-Authorization:",StringComparison.OrdinalIgnoreCase)) throw new Exception("Unexpected credentials at local mock");
                        }
                        if(length<0 || length>4096) throw new Exception("Mock body limit exceeded");
                        byte[] body=new byte[length]; int offset=0;
                        while(offset<length) { int got=stream.Read(body,offset,length-offset); if(got==0) throw new Exception("Truncated upload"); offset+=got; }
                        bool ok=lines[0]=="POST /v1/dashboard-success HTTP/1.1";
                        if(ok && Encoding.UTF8.GetString(body)!=Payload) throw new Exception("Upload changed in transit");
                        if(!ok && lines[0]!="GET /v1/window-smoke-error HTTP/1.1") throw new Exception("Unexpected local mock target: "+lines[0]);
                        byte[] data=Encoding.UTF8.GetBytes(ok?Response:"bad target");
                        byte[] head=Encoding.ASCII.GetBytes("HTTP/1.1 "+(ok?"200 OK":"400 Bad Request")+"\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: "+data.Length+"\r\nConnection: close\r\n\r\n");
                        stream.Write(head,0,head.Length); stream.Write(data,0,data.Length);
                        Interlocked.Add(ref uploaded,length);
                        if(ok) Interlocked.Increment(ref successes); else Interlocked.Increment(ref errors);
                    }
                }
            } catch(SocketException) { if(!stopping) throw; }
              catch(ObjectDisposedException) { if(!stopping) throw; }
        });
    }
    public static void Success(int port) {
        var r=(HttpWebRequest)WebRequest.Create("http://127.0.0.1:"+port+"/dashboard-success");
        r.Proxy=null; r.Method="POST"; r.Timeout=3000; r.ReadWriteTimeout=3000;
        r.KeepAlive=false; r.AllowAutoRedirect=false; r.ServicePoint.Expect100Continue=false;
        r.ContentType="application/json; charset=utf-8";
        byte[] bytes=Encoding.UTF8.GetBytes(Payload); r.ContentLength=bytes.Length;
        using(var stream=r.GetRequestStream()) stream.Write(bytes,0,bytes.Length);
        using(var response=(HttpWebResponse)r.GetResponse())
        using(var reader=new StreamReader(response.GetResponseStream(),Encoding.UTF8)) {
            if(response.StatusCode!=HttpStatusCode.OK || reader.ReadToEnd()!=Response) throw new Exception("Local success status/body mismatch");
        }
    }
    public void Check() { if(worker.IsFaulted) worker.GetAwaiter().GetResult(); }
    public void Dispose() {
        stopping=true; listener.Stop();
        if(!worker.Wait(4000)) throw new Exception("Mock cleanup exceeded four seconds");
        worker.GetAwaiter().GetResult();
    }
}
'@)
$joined = $declarations -join "`n"
$usingPattern = '(?m)^using [^;\r\n]+;\r?$'
$usings = @([regex]::Matches($joined,$usingPattern) | ForEach-Object { $_.Value.Trim() } | Select-Object -Unique)
$combined = ($usings -join "`n") + "`n" + [regex]::Replace($joined,$usingPattern,'')
Add-Type -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll,Accessibility.dll -TypeDefinition $combined
if ($ValidateOnly) {
    Write-Output 'PASS: declaration extraction and combined C# compilation only; no build, EXE, mutex, GUI, mock or acceptance test run'
    return
}

$session=[Diagnostics.Process]::GetCurrentProcess().SessionId
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$mutexName="Local\AIRelay.$sid.$session.v1"
$child=$null; $childHandle=[IntPtr]::Zero; $childStart=$null; $ownedPid=0
$window=[IntPtr]::Zero; $mock=$null; $temp=$null; $failure=$null
$previousDpi=[IntPtr]::Zero; $stage='preflight'
$overall=[Diagnostics.Stopwatch]::StartNew()
$checks=[Collections.Generic.List[object]]::new()
$samples=[Collections.Generic.List[object]]::new()
$shots=[Collections.Generic.List[object]]::new()
$cleanupErrors=[Collections.Generic.List[string]]::new()
$expectedSuccess=0; $expectedError=0
$culture=[Globalization.CultureInfo]::InvariantCulture
function Record([string]$Name,[object]$Value) {
    $checks.Add([pscustomobject]@{ Check=$Name; Value=$Value; Seconds=[Math]::Round($overall.Elapsed.TotalSeconds,3) })
    Write-Host ("CHECK {0}: {1}" -f $Name,($Value | ConvertTo-Json -Depth 7 -Compress))
}
function Assert-NoOtherProcess {
    $others=@(Get-Process -Name 'ai-relay' -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $ownedPid })
    if ($others.Count -gt 0) { throw "BLOCKED: other ai-relay PID(s) $($others.Id -join ','); no input/IPC/kill sent to them" }
}
function Assert-NoInstance {
    $existing=$null
    try { $existing=[Threading.Mutex]::OpenExisting($mutexName) }
    catch [Threading.WaitHandleCannotBeOpenedException] { }
    if ($null -ne $existing) { $existing.Dispose(); throw 'BLOCKED: existing same-user/session mutex; no launch, IPC or input sent' }
    Assert-NoOtherProcess
}
function Assert-Identity {
    if ($null -eq $child -or $child.HasExited) { throw 'Owned child missing/exited; refusing automation/termination' }
    [StatusBarNative]::CheckProcess($childHandle,[uint32]$ownedPid)
    if ($child.Id -ne $ownedPid -or $child.Handle -ne $childHandle -or $child.StartTime -ne $childStart -or $child.Path -ine $exe) {
        throw 'Pinned child PID/handle/start time/path changed; refusing automation/termination'
    }
}
function Assert-Child {
    Assert-NoOtherProcess
    Assert-Identity
    if ($overall.Elapsed.TotalSeconds -ge 240) { throw "Overall 240-second budget exceeded at $stage" }
    if ($null -ne $mock) { $mock.Check() }
}
function Control([int]$Id) {
    Assert-Child
    [ProxyUiSmoke]::CheckOwner($window,[uint32]$ownedPid)
    $h=[ProxyUiSmoke]::GetDlgItem($window,$Id)
    [ProxyUiSmoke]::CheckOwner($h,[uint32]$ownedPid)
    return $h
}
function Text([int]$Id) { [ClarityNative]::Text((Control $Id),[uint32]$ownedPid) }
function Visible([int]$Id) { [ProxyUiSmoke]::IsWindowVisible((Control $Id)) }
function Enabled([int]$Id) { [ProxyUiSmoke]::IsWindowEnabled((Control $Id)) }
function Message([int]$Id,[uint32]$Code,[uint32]$W=0,[int]$L=0) { [ModernNative]::Message((Control $Id),[uint32]$ownedPid,$Code,$W,$L) }
function Wait-State([string]$Name,[scriptblock]$Test,[int]$Seconds=12) {
    $timer=[Diagnostics.Stopwatch]::StartNew()
    while ($timer.Elapsed.TotalSeconds -lt $Seconds) {
        Assert-Child
        if (& $Test) { return }
        Start-Sleep -Milliseconds 60
    }
    throw "Bounded wait expired at $stage / $Name"
}
function Click([int]$Id) {
    $h=Control $Id
    if ([ProxyUiSmoke]::Class($h) -ne 'Button' -or -not (Visible $Id) -or -not (Enabled $Id)) { throw "Native button $Id not usable" }
    [ModernNative]::Click($h,[uint32]$ownedPid)
}
function Select-Page([int]$Index,[int]$Expected) {
    [ModernNative]::ClickTab($childHandle,[uint32]$ownedPid,(Control 2100),$Index)
    Wait-State "page $Index / $Expected visible" { (Message 2100 0x130B) -eq $Index -and (Visible $Expected) }
}
function Assert-ConfigUnchanged {
    if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($config)) -cne $configBytes) { throw 'config.toml changed without Save' }
    if (Test-Path -LiteralPath (Join-Path $temp 'config.toml.bak')) { throw 'Unexpected config backup: an implicit Save may have occurred' }
}
function Parse-Dashboard([string]$Value) {
    $pattern='累计请求 (\d+) 次.*?错误 (\d+) 次（非成功率）；活动请求 (\d+) 个.*?活动隧道 (\d+) 个；入站连接 (\d+) 个.*?下载 (\d+) B；上传 (\d+) B。.*?会话时长 ([\d.]+) 秒'
    $m=[regex]::Match($Value,$pattern)
    if (-not $m.Success) { throw "2109 missing exact real counters/units: $Value" }
    $rates=[regex]::Match($Value,'最新采样区间平均：请求 ([^，]+) 次/s，错误 ([^，]+) 次/s，上传 ([^，]+) B/s，下载 ([^。]+) B/s')
    $r=$null
    if ($rates.Success) {
        $r=@(1..4 | ForEach-Object { [double]::Parse($rates.Groups[$_].Value,$culture) })
        foreach ($v in $r) { if ([double]::IsNaN($v) -or [double]::IsInfinity($v) -or $v -lt 0) { throw 'Invalid rate' } }
    } elseif ($Value -notmatch '首个样本|尚无快照|采样间隔超过 5 秒|计数饱和|计数／时长回退') { throw "Unrecognized rate metadata: $Value" }
    return [pscustomobject]@{
        Requests=[long]$m.Groups[1].Value; Errors=[long]$m.Groups[2].Value
        Active=[long]$m.Groups[3].Value; Tunnels=[long]$m.Groups[4].Value; Inbound=[long]$m.Groups[5].Value
        Download=[long]$m.Groups[6].Value; Upload=[long]$m.Groups[7].Value
        Uptime=[double]::Parse($m.Groups[8].Value,$culture); Rates=$r; Text=$Value
    }
}
function Format-Bytes([long]$Value) {
    $n=[double]$Value; $units=@('B','KiB','MiB','GiB','TiB'); $i=0
    while ($n -ge 1024 -and $i -lt 4) { $n/=1024; $i++ }
    return $n.ToString('F1',$culture)+' '+$units[$i]
}
function Read-Snapshot {
    # A timer can update siblings between WM_GETTEXT calls. Only compare a stable pair.
    for ($attempt=0;$attempt -lt 8;$attempt++) {
        $legacy=Text 2107; $meta=Text 2109
        if ($legacy -cne (Text 2107) -or $meta -cne (Text 2109)) { continue }
        $s=Parse-Dashboard $meta
        foreach ($needle in @("活动请求：$($s.Active)    活动隧道：$($s.Tunnels)    入站连接：$($s.Inbound)","累计请求：$($s.Requests)    错误：$($s.Errors)","上传正文／隧道：$(Format-Bytes $s.Upload)    下载正文／隧道：$(Format-Bytes $s.Download)")) {
            if (-not $legacy.Contains($needle)) { throw "2107 and 2109 disagree: expected '$needle'; legacy=$legacy; metadata=$meta" }
        }
        $t=[regex]::Match($legacy,'程序会话时长：(\d+) 天 (\d+):(\d+):(\d+)')
        if (-not $t.Success) { throw '2107 session uptime missing' }
        $seconds=[long]$t.Groups[1].Value*86400+[long]$t.Groups[2].Value*3600+[long]$t.Groups[3].Value*60+[long]$t.Groups[4].Value
        # Metadata rounds to milliseconds; 0.9996 may legitimately print 1.000.
        if ($s.Uptime-$seconds -lt -0.001 -or $s.Uptime-$seconds -gt 1.001) { throw '2107/2109 uptime differs by more than rounding' }
        $s | Add-Member -NotePropertyName Legacy -NotePropertyValue $legacy
        return $s
    }
    throw 'Could not read stable overview texts in eight bounded attempts'
}
function Observe {
    $s=Read-Snapshot
    if ($samples.Count -eq 0 -or $samples[$samples.Count-1].Text -cne $s.Text) {
        if ($samples.Count -ge 512) { throw 'Observation bound exceeded' }
        $samples.Add($s)
    }
    return $s
}
function Assert-Totals([object]$s) {
    $requests=$expectedSuccess+$expectedError
    $upload=[long]$expectedSuccess*[DashboardMock]::UploadBytes
    $download=[long]$expectedSuccess*[DashboardMock]::DownloadBytes+[long]$expectedError*10
    if ($s.Requests -ne $requests -or $s.Errors -ne $expectedError -or $s.Upload -ne $upload -or $s.Download -ne $download -or $s.Active -ne 0 -or $s.Tunnels -ne 0) {
        throw "Actual counters differ from completed local HTTP bodies: expected req=$requests err=$expectedError up=$upload down=$download; actual=$($s | ConvertTo-Json -Compress)"
    }
    if ($mock.Successes -ne $expectedSuccess -or $mock.Errors -ne $expectedError -or $mock.Uploaded -ne $upload) { throw 'Independent local mock counts/received upload bytes mismatch' }
}
function Send-Round {
    Assert-Child
    [DashboardMock]::Success($port); $script:expectedSuccess++
    Assert-Child
    [DashboardMock]::Success($port); $script:expectedSuccess++
    # A real upstream HTTP 400, read to EOS. Never infer errors from 1-success-rate.
    [WindowSmoke]::RequestError($port); $script:expectedError++
}
function Wait-Totals {
    Wait-State 'completed request counters and all body bytes' {
        $s=Observe
        $s.Requests -eq ($expectedSuccess+$expectedError) -and $s.Errors -eq $expectedError -and
        $s.Upload -eq ([long]$expectedSuccess*[DashboardMock]::UploadBytes) -and
        $s.Download -eq ([long]$expectedSuccess*[DashboardMock]::DownloadBytes+[long]$expectedError*10) -and $s.Active -eq 0
    }
    $s=Observe; Assert-Totals $s; return $s
}
function Assert-Buttons([bool]$Running) {
    foreach ($id in @(2102,2103,2104,2105,2106)) {
        $h=Control $id
        if ([ProxyUiSmoke]::Class($h) -ne 'Button' -or -not (Visible $id) -or ([ProxyUiSmoke]::GetWindowLongW($h,-16) -band 0x10000) -eq 0) { throw "Missing native focusable button $id" }
    }
    if ((Enabled 2102) -eq $Running -or (Enabled 2103) -ne $Running -or -not (Enabled 2104) -or (Enabled 2105) -ne $Running -or -not (Enabled 2106)) { throw "Wrong overview button availability (running=$Running)" }
    Record 'native BUTTON availability (copy/directory not invoked: no clipboard/Explorer side effects)' @{ Running=$Running; Start=(Enabled 2102); Stop=(Enabled 2103); Restart=(Enabled 2104); Copy=(Enabled 2105); Directory=(Enabled 2106) }
}
function Assert-Layout([string]$Name) {
    Assert-Child
    $dpi=[ClarityNative]::GetDpiForWindow($window); $factor=96.0/$dpi
    $client=[StatusBarNative]::Client($window,[uint32]$ownedPid)
    $outer=[StatusBarNative]::Outer($window,[uint32]$ownedPid)
    $work=[WindowSmoke]::Work($window,[uint32]$ownedPid)
    if ($outer.Left -lt $work.Left -or $outer.Top -lt $work.Top -or $outer.Right -gt $work.Right -or $outer.Bottom -gt $work.Bottom) { throw "$Name exceeds physical monitor work area" }
    $rects=[Collections.Generic.List[object]]::new()
    foreach ($h in [ModernNative]::DirectChildren($window,[uint32]$ownedPid)) {
        if (-not [ProxyUiSmoke]::IsWindowVisible($h)) { continue }
        $r=[ModernNative]::Bounds($h,$window,[uint32]$ownedPid)
        if ($r.Width -le 0 -or $r.Height -le 0) { continue }
        $id=[ClarityNative]::GetDlgCtrlID($h)
        if ($r.Left -lt 0 -or $r.Top -lt 0 -or $r.Right -gt $client.Width -or $r.Bottom -gt $client.Height) { throw "$Name visible child clipped: ID=$id bounds=$r client=$client" }
        $rects.Add([pscustomobject]@{ ID=$id; Class=[ProxyUiSmoke]::Class($h); Rect=$r; Text=[ClarityNative]::Text($h,[uint32]$ownedPid) })
    }
    for ($a=0;$a -lt $rects.Count;$a++) { for ($b=$a+1;$b -lt $rects.Count;$b++) {
        $ra=$rects[$a].Rect; $rb=$rects[$b].Rect
        if ($ra.Left -lt $rb.Right -and $ra.Right -gt $rb.Left -and $ra.Top -lt $rb.Bottom -and $ra.Bottom -gt $rb.Top) { throw "$Name visible siblings overlap: ID=$($rects[$a].ID) [$ra] and ID=$($rects[$b].ID) [$rb]" }
    } }
    $dash=[StatusBarNative]::InParent((Control 2109),$window,[uint32]$ownedPid)
    $head=[StatusBarNative]::InParent((Control 2108),$window,[uint32]$ownedPid)
    if ([Math]::Abs($dash.Top*$factor-282) -gt 1 -or [Math]::Abs($dash.Height*$factor-($client.Height*$factor-366)) -gt 2 -or [Math]::Abs($head.Top*$factor-108) -gt 1 -or [Math]::Abs($head.Bottom*$factor-208) -gt 1) { throw "$Name dashboard/header geometry differs from expected DIP layout" }
    foreach ($id in @(2102,2103,2104,2105,2106)) {
        $r=[StatusBarNative]::InParent((Control $id),$window,[uint32]$ownedPid)
        if ([Math]::Abs($r.Top*$factor-220) -gt 1 -or [Math]::Abs($r.Height*$factor-36) -gt 1) { throw "$Name button $id not on the y=220 DIP row" }
    }
    $status=@($rects | Where-Object { $_.Class -eq 'Static' -and $_.Text.StartsWith('代理状态：') })
    $title=@($rects | Where-Object { $_.Class -eq 'Static' -and $_.ID -notin @(2108,2109) -and $_.Rect.Top*$factor -ge 60 -and $_.Rect.Bottom*$factor -le 96 -and -not [string]::IsNullOrWhiteSpace($_.Text) })
    if ($status.Count -ne 1 -or $title.Count -ne 1 -or $dash.Bottom -gt $status[0].Rect.Top) { throw "$Name title/status missing or dashboard covers status" }
    if ((Visible 2107) -or $dash.Width*$factor -lt 695.5 -or $dash.Height*$factor -lt 349.5) { throw "$Name dashboard below 696x350 DIP or legacy 2107 visible; client=$client dashboard=$dash dpi=$dpi" }
    $panelWidth=($dash.Width-12/$factor)/2
    $plotWidth=($panelWidth-80/$factor)*$factor
    $plotHeight=($dash.Height-296/$factor)*$factor
    Record "$Name physical client/minimum/plot geometry and nonoverlapping visible children" @{ DPI=$dpi; Client=$client; Dashboard=$dash; Work=$work; DashboardDIP=@(($dash.Width*$factor),($dash.Height*$factor)); PlotDIP=@($plotWidth,$plotHeight); Children=@($rects.ToArray()) }
}
function Resize-Overview([bool]$Minimum) {
    $dpi=[ClarityNative]::GetDpiForWindow($window); $scale=$dpi/96.0
    $work=[WindowSmoke]::Work($window,[uint32]$ownedPid)
    $minW=[int][Math]::Round(760*$scale); $minH=[int][Math]::Round(880*$scale)
    if ($work.Right-$work.Left -lt $minW -or $work.Bottom-$work.Top -lt $minH) { throw "Current work area cannot fit the specified 760x880 DIP minimum at DPI $dpi; no clipping acceptance claimed" }
    $w=$minW; $h=$minH
    if (-not $Minimum) { $w=[Math]::Min($work.Right-$work.Left,[int][Math]::Round(940*$scale)); $h=[Math]::Min($work.Bottom-$work.Top,[int][Math]::Round(1020*$scale)) }
    [DashboardNative]::Resize($window,[uint32]$ownedPid,$work.Left,$work.Top,$w,$h)
    Wait-State 'physical resize completed' { $r=[StatusBarNative]::Outer($window,[uint32]$ownedPid); $r.Width -eq $w -and $r.Height -eq $h }
    Start-Sleep -Milliseconds 150
}
function Capture([string]$Name,[IntPtr]$Handle,[switch]$Outer) {
    Assert-Child
    if (-not [ProxyUiSmoke]::IsWindowVisible($Handle)) { throw "Hidden screenshot target $Name" }
    $shot=[ModernNative]::Capture($Handle,[uint32]$ownedPid,(Join-Path $output "$Name.png"),(-not $Outer),(New-Object StatusBarNative+Rect))
    $shots.Add([pscustomobject]@{ Name=$Name; DPI=[ClarityNative]::GetDpiForWindow($Handle); Shot=$shot })
    return $shot
}
function Theme([int]$Index) {
    Select-Page 3 2039
    [void](Message 2039 0x14E 5)
    [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,[uint32](2039 -bor 0x10000))
    Wait-State 'appearance theme segment visible' { Visible 2030 }
    if ([ProxyUiSmoke]::Class((Control 2030)) -ne 'SysTabControl32') { throw '2030 must be native segmented SysTabControl32' }
    $frozen=Text 2109
    [ModernNative]::ClickTab($childHandle,[uint32]$ownedPid,(Control 2030),$Index)
    Wait-State "native theme $Index selected" { (Message 2030 0x130B) -eq $Index -and (Message 2030 0x147) -eq $Index }
    Assert-ConfigUnchanged
    if ((Text 2109) -cne $frozen) { throw 'Hidden overview metadata changed during theme preview' }
    Select-Page 0 2109
    [void](Wait-Totals)
    [DashboardNative]::Invalidate((Control 2109),[uint32]$ownedPid)
    Start-Sleep -Milliseconds 160
}

# Before sockets, temp files, launch or IPC. ValidateOnly never gets here.
Assert-NoInstance
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "Build unavailable: $([IO.Path]::GetFullPath($ExePath)). Build separately and rerun; this script never runs Cargo." }
$exe=(Get-Item -LiteralPath $ExePath).FullName
$outputRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\target\overview-dashboard-screenshots'))
$output=Join-Path $outputRoot ([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')+'-'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($output)
try {
    $previousDpi=[StatusBarNative]::EnterDpi()
    Assert-NoInstance
    $temp=Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-overview-dashboard-'+[Guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($temp)
    $mock=[DashboardMock]::new(); $port=[ProxyUiSmoke]::FreePort()
    if ($port -eq $mock.Port) { throw 'Ephemeral loopback ports collided; safely rerun' }
    $config=Join-Path $temp 'config.toml'
    $toml="schema_version = 1`n[upstream]`nurl = 'http://127.0.0.1:$($mock.Port)/v1'`n[listen]`naddress = '127.0.0.1'`nport = $port`n[proxy]`nauto_start = false`nforward_enabled = false`nconnect_enabled = false`n[ui]`ntheme = 'light'`nremember_window_geometry = false`n[system]`nlogin_auto_start = false`n"
    [IO.File]::WriteAllText($config,$toml,[Text.UTF8Encoding]::new($false))
    $configBytes=[Convert]::ToBase64String([IO.File]::ReadAllBytes($config))
    $stage='launch exact private child'
    Assert-NoInstance
    $child=Start-Process -FilePath $exe -WorkingDirectory $temp -ArgumentList @('--config-dir',('"'+$temp+'"')) -PassThru
    $childHandle=$child.Handle; $childStart=$child.StartTime; $ownedPid=$child.Id
    $identityTimer=[Diagnostics.Stopwatch]::StartNew()
    do {
        Assert-NoOtherProcess
        [StatusBarNative]::CheckProcess($childHandle,[uint32]$ownedPid)
        $child.Refresh()
        if ($child.Path -ieq $exe) { break }
        if ($identityTimer.Elapsed.TotalSeconds -ge 5) { throw 'Child executable metadata unavailable after five seconds; no GUI input sent' }
        Start-Sleep -Milliseconds 50
    } while ($true)
    Wait-State 'owned main HWND' { $script:window=[ProxyUiSmoke]::Find([uint32]$ownedPid); $window -ne [IntPtr]::Zero }
    Wait-State 'first actual metrics snapshot' { (Text 2109).Contains('本次程序会话：累计请求') }
    $first=Observe
    if ($first.Text.Contains('首个样本')) {
        if ($null -ne $first.Rates) { throw 'First sample invented zero rates' }
        Record 'first sample unknown, not zero' $first
    } else { Record 'SKIP first <=1s unknown window (already elapsed; not manufactured)' $first.Text }
    Wait-State 'saved private config loaded / stopped' { (Enabled 2102) -and -not (Enabled 2103) -and (Enabled 2104) }
    Record 'pinned exact child / EXE identity' @{ PID=$ownedPid; Start=$childStart.ToString('o'); Exe=$exe; SHA256=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash; LastWriteUTC=[IO.File]::GetLastWriteTimeUtc($exe).ToString('o') }
    if ([ProxyUiSmoke]::Class((Control 2109)) -ne 'Static' -or ([ProxyUiSmoke]::GetWindowLongW((Control 2109),-16) -band 31) -ne 13 -or (Visible 2107)) { throw 'Expected 2109 SS_OWNERDRAW STATIC and hidden legacy 2107' }
    Resize-Overview $true
    Assert-Layout 'minimum-760x880'
    Assert-Buttons $false
    Assert-Totals (Observe)

    $stage='at least nine seconds actual sampling / three 1.2s-spaced local request rounds'
    Click 2102
    Wait-State 'running native stop enabled / actual header' { (Enabled 2103) -and (Text 2108).Contains('正在运行') }
    Assert-Buttons $true
    $sampling=[Diagnostics.Stopwatch]::StartNew(); $round=0; $nextRound=2.4
    $sampleStart=Observe
    while ($sampling.Elapsed.TotalSeconds -lt 9) {
        Assert-Child
        if ($round -lt 3 -and $sampling.Elapsed.TotalSeconds -ge $nextRound) {
            Send-Round; $round++
            $nextRound=$sampling.Elapsed.TotalSeconds+1.2
            Record "real local HTTP round $round" @{ Success=$expectedSuccess; Http400=$expectedError; At=$sampling.Elapsed.TotalSeconds }
        }
        [void](Observe)
        Start-Sleep -Milliseconds 90
    }
    $after=Wait-Totals
    if ($round -ne 3 -or $after.Uptime-$sampleStart.Uptime -lt 8) { throw 'Insufficient real uptime sampling (minimum eight seconds)' }
    $seen=@($false,$false,$false,$false); $matched=0
    for ($i=0;$i -lt $samples.Count;$i++) {
        $now=$samples[$i]
        if ($null -eq $now.Rates) { continue }
        for ($k=0;$k -lt 4;$k++) { if ($now.Rates[$k] -gt 0) { $seen[$k]=$true } }
        if ($now.Rates[0] -le 0) { continue }
        # WM_GETTEXT has millisecond uptime precision, rates have two decimals.
        # Match only actual observed endpoints, allowing rounding (not a synthetic timer).
        $consistent=$false
        for ($j=$i-1;$j -ge 0;$j--) {
            $before=$samples[$j]; $dt=$now.Uptime-$before.Uptime
            if ($dt -gt 5.01) { break }
            if ($dt -lt 0.999) { continue }
            $deltas=@(($now.Requests-$before.Requests),($now.Errors-$before.Errors),($now.Upload-$before.Upload),($now.Download-$before.Download))
            $ok=$true
            for ($k=0;$k -lt 4;$k++) {
                $expected=$deltas[$k]/$dt
                if ($expected -lt 0 -or [Math]::Abs($now.Rates[$k]-$expected) -gt (0.016+[Math]::Abs($expected)*0.003)) { $ok=$false; break }
            }
            if ($ok) { $consistent=$true; break }
        }
        if ($consistent) { $matched++ }
    }
    if ($seen -contains $false -or $matched -lt 2) { throw "Did not observe real positive request/error/upload/download interval rates with >=2 counter/uptime matches; seen=$seen matches=$matched" }
    Record 'actual >=8s uptime, positive rates and observed delta/elapsed consistency' @{ RealSeconds=$sampling.Elapsed.TotalSeconds; UptimeDelta=$after.Uptime-$sampleStart.Uptime; Positive=$seen; MatchingIntervals=$matched; Snapshot=$after }

    $stage='stats page: hidden texts frozen, backend requests/sampling continue'
    Select-Page 1 2300
    Wait-State 'overview controls hidden on stats' { -not (Visible 2109) -and -not (Visible 2108) -and -not (Visible 2102) }
    Start-Sleep -Milliseconds 150
    $frozen7=Text 2107; $frozen9=Text 2109; $frozen8=Text 2108
    $hidden=[Diagnostics.Stopwatch]::StartNew()
    for ($i=0;$i -lt 3;$i++) {
        Send-Round
        $pause=[Diagnostics.Stopwatch]::StartNew()
        while ($pause.Elapsed.TotalSeconds -lt 1.2) {
            Assert-Child
            if ((Text 2107) -cne $frozen7 -or (Text 2109) -cne $frozen9 -or (Text 2108) -cne $frozen8) { throw 'Hidden overview text refreshed on stats page' }
            Start-Sleep -Milliseconds 100
        }
    }
    Select-Page 0 2109
    $returned=Wait-Totals
    if ($returned.Uptime -le $after.Uptime+3 -or $returned.Text -ceq $frozen9 -or $null -eq $returned.Rates -or $returned.Text.Contains('首个样本')) { throw 'Returning from stats did not preserve cumulative session/sampling metadata' }
    Record 'hidden overview texts frozen while real backend accumulates; return catches up' @{ HiddenSeconds=$hidden.Elapsed.TotalSeconds; Before=$after; After=$returned }

    $stage='three native theme segments / unchanged config and session snapshots'
    $light=$null; $dark=$null
    foreach ($index in @(0,1,2)) {
        $name=@('system','light','dark')[$index]
        $before=Observe
        Theme $index
        $current=Observe; Assert-Totals $current
        if ($current.Uptime -lt $before.Uptime) { throw 'Theme reset actual uptime/session' }
        Assert-Buttons $true
        Assert-Layout "$name-minimum"
        if ($index -eq 0) { Record 'system theme selected by native segment; retained real session' $current; continue }
        [void](Capture "overview-$name-minimum" $window -Outer)
        $shot=Capture "dashboard-$name-minimum" (Control 2109)
        $proof=[DashboardNative]::Inspect($shot.Path,[ClarityNative]::GetDpiForWindow((Control 2109)),($index -eq 2))
        foreach ($p in $proof) {
            if ($p.Name -like 'card-*' -and $p.Colored -lt 12) { throw "Missing rough colored card evidence: $($p.Name)" }
            if ($p.Name -like '*legend' -and ($p.Blue -lt 4 -or ($p.Name -like 'chart-0-*' -and $p.Red -lt 4) -or ($p.Name -like 'chart-1-*' -and $p.Teal -lt 4))) { throw "Missing two colored series legends: $($p.Name)" }
            if ($p.Name -like '*units' -and $p.Ink -lt 4) { throw "No rough unit text ink: $($p.Name)" }
            if ($p.Name -like '*plot' -and $p.Colored -lt 4) { throw "No actual sampled curve pixels: $($p.Name)" }
        }
        Record "$name physical dashboard rough pixel evidence vs exact WM_GETTEXT" @{ Screenshot=$shot; Regions=$proof; Metadata=$current.Text; ExpectedLegends=@('请求 / 错误：次/s','上传 / 下载：B/s'); OCR='Not performed; ink/color presence does not identify glyphs or numerical values' }
        if ($index -eq 1) { $light=$shot } else { $dark=$shot }
    }
    if ($light.MeanLuminance -lt 150 -or $dark.MeanLuminance -gt 125 -or $light.MeanLuminance-$dark.MeanLuminance -lt 60) { throw 'Light/dark physical surfaces did not change distinctly (high contrast requires separate acceptance)' }
    Resize-Overview $false
    foreach ($index in @(1,2)) {
        $name=@('system','light','dark')[$index]
        Theme $index
        Assert-Totals (Observe)
        Assert-Buttons $true
        Assert-Layout "$name-expanded-within-workarea"
        [void](Capture "overview-$name-expanded" $window -Outer)
        [void](Capture "dashboard-$name-expanded" (Control 2109))
    }

    $stage='bounded theme/draw GDI check (six one-second rounds, not long stability)'
    $gdiBefore=[StatusBarNative]::Gdi($childHandle,[uint32]$ownedPid)
    $gdi=[Collections.Generic.List[uint32]]::new()
    foreach ($index in @(0,1,2,0,1,2)) {
        Theme $index
        $draw=[Diagnostics.Stopwatch]::StartNew()
        while ($draw.Elapsed.TotalSeconds -lt 1.1) { [void](Observe); Start-Sleep -Milliseconds 100 }
        $gdi.Add([StatusBarNative]::Gdi($childHandle,[uint32]$ownedPid))
    }
    $gdiAfter=[StatusBarNative]::Gdi($childHandle,[uint32]$ownedPid)
    $peak=($gdi | Measure-Object -Maximum).Maximum
    if ([long]$gdiAfter-[long]$gdiBefore -gt 16 -or $peak-[long]$gdiBefore -gt 48) { throw "Unbounded-looking short GDI change: before=$gdiBefore after=$gdiAfter peak=$peak" }
    Record 'short warmed theme/timer GDI bounds (not long-run proof)' @{ Before=$gdiBefore; Counts=@($gdi.ToArray()); After=$gdiAfter; AllowedFinalGrowth=16; AllowedPeakGrowth=48 }

    $stage='native restart keeps counters; native stop leaves actual idle zero rates'
    $beforeRestart=Observe
    Click 2104
    Wait-State 'native restart completed' { (Enabled 2103) -and (Enabled 2104) -and (Text 2108).Contains('正在运行') }
    [void](Wait-Totals)
    Send-Round
    $beforeStop=Wait-Totals
    if ($beforeStop.Uptime -lt $beforeRestart.Uptime) { throw 'Restart reset session uptime' }
    Click 2103
    Wait-State 'stopped native controls and summary' { (Enabled 2102) -and -not (Enabled 2103) -and (Text 2108).Contains('已停止') }
    if (-not [ProxyUiSmoke]::PortClosed($port)) { throw 'Private listener still open after stop' }
    Wait-State 'real idle rates all zero after stop' {
        $s=Observe
        $null -ne $s.Rates -and @($s.Rates | Where-Object { $_ -ne 0 }).Count -eq 0 -and $s.Uptime -ge $beforeStop.Uptime+2
    }
    $stopped=Observe; Assert-Totals $stopped
    Assert-Buttons $false
    Assert-Layout 'stopped-expanded'
    [void](Capture 'overview-dark-stopped' $window -Outer)
    Record 'stop preserves exact counters/bytes, uptime advances and sampled idle becomes real zero' $stopped
    Assert-ConfigUnchanged
    $stage='normal owned-child exit'
    Assert-Child
    [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,1002)
    if (-not $child.WaitForExit(38000) -or $child.ExitCode -ne 0) { throw 'Normal 1002 exit failed/exceeded 38 seconds' }
    Assert-ConfigUnchanged
    Record 'normal 1002 exit / private config bytes unchanged' $ownedPid
} catch {
    $failure=$_
    Write-Warning ("FAIL stage={0}; {1}; {2}" -f $stage,$_.Exception.Message,$_.InvocationInfo.PositionMessage)
} finally {
    $exited=$null -eq $child
    if ($null -ne $child) {
        try {
            if (-not $child.HasExited) {
                # Even if a foreign instance appeared, cleanup only addresses the
                # original live process handle and owner-checked child HWND.
                Assert-Identity
                if ($window -eq [IntPtr]::Zero) { $window=[ProxyUiSmoke]::Find([uint32]$ownedPid) }
                if ($window -ne [IntPtr]::Zero) { [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,1002) }
                if (-not $child.WaitForExit(38000)) {
                    Assert-Identity
                    $child.Kill()
                    if (-not $child.WaitForExit(5000)) { throw 'Pinned child termination exceeded five seconds' }
                    $cleanupErrors.Add('Owned child required forced termination; normal exit NOT passed')
                }
            }
            $exited=$child.HasExited
        } catch { $cleanupErrors.Add($_.Exception.Message) }
        finally { $child.Dispose(); $child=$null }
    }
    if ($null -ne $mock) { try { $mock.Dispose() } catch { $cleanupErrors.Add("Mock cleanup: $($_.Exception.Message)") } }
    if ($previousDpi -ne [IntPtr]::Zero) { [StatusBarNative]::RestoreDpi($previousDpi) }
    if ($null -ne $temp -and $exited) {
        for ($attempt=0;$attempt -lt 10;$attempt++) {
            try { [IO.Directory]::Delete($temp,$true); break }
            catch { if ($attempt -eq 9) { $cleanupErrors.Add("Temp retained at $temp : $($_.Exception.Message)") } else { Start-Sleep -Milliseconds 150 } }
        }
    } elseif ($null -ne $temp) { $cleanupErrors.Add("Child exit unconfirmed; temp retained at $temp") }
    $detail=$null
    if ($null -ne $failure) { $detail=[pscustomobject]@{ Stage=$stage; Message=$failure.Exception.Message; Position=$failure.InvocationInfo.PositionMessage; Stack=$failure.ScriptStackTrace } }
    $report=[pscustomobject]@{
        Passed=($null -eq $failure -and $cleanupErrors.Count -eq 0); Exe=$exe; PID=$ownedPid
        Failure=$detail; CleanupErrors=@($cleanupErrors.ToArray()); Checks=@($checks.ToArray())
        ActualSamples=@($samples.ToArray()); Screenshots=@($shots.ToArray()); Seconds=$overall.Elapsed.TotalSeconds
        Limits='Current monitor physical DPI only. Pixel checks are rough, not OCR/legend glyph proof. No multi-monitor/high-contrast/screen-reader/physical tray acceptance. First <=1s may be skipped. No induced gap/reset/saturation or 120s retention acceptance: those need Rust unit tests. Known 760x880 DIP minimum layout, not an interactive min-track proof. Copy/directory availability only (no clipboard/Explorer). Short GDI check, not long stability. No Cargo, old script execution, real API/key, Startup/firewall/system proxy changes, global input or IPC to another instance.'
    }
    [IO.File]::WriteAllText((Join-Path $output 'results.json'),($report | ConvertTo-Json -Depth 14),[Text.UTF8Encoding]::new($true))
    Write-Output "Overview dashboard artifacts: $output"
}
if ($null -ne $failure) { throw $failure }
if ($cleanupErrors.Count -gt 0) { throw ($cleanupErrors -join '; ') }
Write-Output 'PASS: actual sampled dashboard rates/counters/body bytes, hidden-page freeze/catch-up, native themes/buttons, physical light/dark captures, minimum/resize layout, bounded GDI and normal owned-child exit'
