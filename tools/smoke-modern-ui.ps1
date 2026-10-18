# Windows PowerShell 5.1 x64, interactive Windows 10/11 desktop.
# Only this script's pinned child and loopback mock are used. Never runs Cargo.
[CmdletBinding()]
param(
    [string]$ExePath,
    [switch]$ValidateOnly
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([IntPtr]::Size -ne 8) { throw 'Use 64-bit powershell.exe -NoProfile' }
if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $PSScriptRoot '..\target\modern-ui-validation\target\debug\ai-relay.exe'
}

# Declarations ONLY: no dot-sourcing, Invoke-Expression, or execution of old tests.
$parts = @(
    @{ File='smoke-proxy-ui.ps1'; Pattern="(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@" },
    @{ File='smoke-ui-clarity.ps1'; Pattern="(?s)Add-Type -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll -TypeDefinition @'\r?\n(.*?)\r?\n'@" },
    @{ File='smoke-status-bar-ui.ps1'; Pattern='(?s)\$statusDeclaration = @''\r?\n(.*?)\r?\n''@' }
)
$declarations = [Collections.Generic.List[string]]::new()
foreach ($part in $parts) {
    $source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot $part.File))
    $match = [regex]::Match($source,$part.Pattern)
    if (-not $match.Success) { throw "Missing declaration in $($part.File); no older script executed" }
    $declarations.Add($match.Groups[1].Value)
}
foreach ($name in @('ProxyUiSmoke','ProxyUiMock','ClarityNative','StatusBarNative','ModernNative')) {
    if ($name -as [type]) { throw "Helper $name already loaded; use a fresh powershell.exe -NoProfile process" }
}
$declarations.Add(@'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
public static class ModernNative {
    [StructLayout(LayoutKind.Sequential)] public struct ComboInfo {
        public uint Size; public StatusBarNative.Rect Item,Button; public uint State;
        public IntPtr Combo,Edit,List;
    }
    public sealed class Shot {
        public string Path; public int Width,Height,Colors;
        public double MeanLuminance,BrightFraction,BrightTileFraction;
    }
    public sealed class PendingProof {
        public bool Seen; public int Attempts; public string[] Frozen;
        public int Date,Zone,Trend,Table,Export;
    }
    [DllImport("user32.dll")] static extern bool GetComboBoxInfo(IntPtr window,ref ComboInfo info);
    [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr window,IntPtr dc,uint flags);
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr window,uint relation);
    [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr OpenProcess(uint access,bool inherit,uint pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr VirtualAllocEx(IntPtr process,IntPtr address,UIntPtr size,uint kind,uint protection);
    [DllImport("kernel32.dll")] static extern bool VirtualFreeEx(IntPtr process,IntPtr address,UIntPtr size,uint kind);
    [DllImport("kernel32.dll")] static extern bool ReadProcessMemory(IntPtr process,IntPtr address,byte[] bytes,UIntPtr size,out UIntPtr read);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr window,uint message,UIntPtr w,IntPtr l,uint flags,uint timeout,out UIntPtr result);
    public static long Message(IntPtr window,uint pid,uint message,uint w,int l) {
        return ClarityNative.Message(window,pid,message,w,l);
    }
    public static ComboInfo Combo(IntPtr window,uint pid) {
        ProxyUiSmoke.CheckOwner(window,pid);
        if(ProxyUiSmoke.Class(window)!="ComboBox") throw new Exception("Expected native ComboBox");
        var info=new ComboInfo { Size=(uint)Marshal.SizeOf(typeof(ComboInfo)) };
        if(!GetComboBoxInfo(window,ref info) || info.Combo!=window || info.Button.Height<=0)
            throw new Exception("GETCOMBOBOXINFO failed/invalid");
        ProxyUiSmoke.CheckOwner(info.List,pid);
        return info;
    }
    public static IntPtr[] DirectChildren(IntPtr parent,uint pid) {
        ProxyUiSmoke.CheckOwner(parent,pid); var found=new List<IntPtr>();
        for(IntPtr h=GetWindow(parent,5);h!=IntPtr.Zero;h=GetWindow(h,2)) {
            if(found.Count>=4096) throw new Exception("Child enumeration exceeded limit");
            ProxyUiSmoke.CheckOwner(h,pid); found.Add(h);
        }
        return found.ToArray();
    }
    public static StatusBarNative.Rect Bounds(IntPtr window,IntPtr parent,uint pid) {
        var r=StatusBarNative.InParent(window,parent,pid);
        if(ProxyUiSmoke.Class(window)=="ComboBox") {
            // A ComboBox's nominal HWND height may include its dropdown. Only its
            // closed selection/arrow band participates in sibling overlap checks.
            var info=Combo(window,pid);
            int closedHeight=Math.Max(info.Item.Bottom,info.Button.Bottom)+Math.Max(0,info.Item.Top);
            // Some native dropdowns have rcItem.Top == 0 and no border inset.
            // Never invent an extra bottom pixel or exceed the actual client.
            var client=StatusBarNative.Client(window,pid);
            r.Bottom=r.Top+Math.Min(client.Height,closedHeight);
        }
        return r;
    }
    public static StatusBarNative.Rect TabItem(IntPtr pinned,uint pid,IntPtr tab,int index) {
        StatusBarNative.CheckProcess(pinned,pid); ProxyUiSmoke.CheckOwner(tab,pid);
        if(ProxyUiSmoke.Class(tab)!="SysTabControl32" || index<0 || index>=Message(tab,pid,0x1304,0,0))
            throw new Exception("Invalid native Tab/item");
        // TCM_GETITEMRECT is >= WM_USER: user32 does NOT marshal RECT pointers.
        // Allocate the output RECT in the pinned child's address space, not ours.
        IntPtr process=OpenProcess(0x101038,false,pid),remote=IntPtr.Zero;
        bool completed=true;
        if(process==IntPtr.Zero) throw new Exception("Cannot open owned child for native Tab RECT");
        try {
            StatusBarNative.CheckProcess(process,pid);
            remote=VirtualAllocEx(process,IntPtr.Zero,new UIntPtr(16),0x3000,4);
            if(remote==IntPtr.Zero) throw new Exception("Tab RECT allocation failed");
            UIntPtr result,read; completed=false;
            if(SendMessageTimeoutW(tab,0x130A,new UIntPtr((uint)index),remote,0x22,2000,out result)==IntPtr.Zero)
                throw new Exception("TCM_GETITEMRECT timeout; target allocation retained until child exit");
            completed=true;
            if(result==UIntPtr.Zero) throw new Exception("TCM_GETITEMRECT failed");
            byte[] bytes=new byte[16];
            if(!ReadProcessMemory(process,remote,bytes,new UIntPtr(16),out read) || read.ToUInt64()!=16)
                throw new Exception("Cannot read complete target Tab RECT");
            return new StatusBarNative.Rect { Left=BitConverter.ToInt32(bytes,0),Top=BitConverter.ToInt32(bytes,4),Right=BitConverter.ToInt32(bytes,8),Bottom=BitConverter.ToInt32(bytes,12) };
        } finally {
            if(remote!=IntPtr.Zero && completed) VirtualFreeEx(process,remote,UIntPtr.Zero,0x8000);
            CloseHandle(process);
        }
    }
    public static void Mouse(IntPtr window,uint pid,int x,int y) {
        ProxyUiSmoke.CheckOwner(window,pid);
        if(!ProxyUiSmoke.IsWindowVisible(window) || !ProxyUiSmoke.IsWindowEnabled(window))
            throw new Exception("Refusing mouse input to hidden/disabled control");
        var r=StatusBarNative.Client(window,pid);
        if(x<0 || y<0 || x>=r.Width || y>=r.Height) throw new Exception("Mouse point outside physical client");
        int point=(y<<16)|(x&65535);
        Message(window,pid,0x200,0,point);
        Message(window,pid,0x201,1,point);
        Message(window,pid,0x202,0,point);
    }
    public static void ClickTab(IntPtr pinned,uint pid,IntPtr tab,int index) {
        var previous=StatusBarNative.EnterDpi();
        try {
            var r=TabItem(pinned,pid,tab,index);
            Mouse(tab,pid,(r.Left+r.Right)/2,(r.Top+r.Bottom)/2);
            // The native Tab emits TCN_SELCHANGE; the application posts its own
            // CBN_SELCHANGE-compatible command. Never fabricate WM_NOTIFY here.
        } finally { StatusBarNative.RestoreDpi(previous); }
    }
    public static void Click(IntPtr window,uint pid) {
        var previous=StatusBarNative.EnterDpi();
        try { var r=StatusBarNative.Client(window,pid); Mouse(window,pid,r.Width/2,r.Height/2); }
        finally { StatusBarNative.RestoreDpi(previous); }
    }
    public static PendingProof ProbePending(IntPtr pinned,uint pid,IntPtr parent) {
        StatusBarNative.CheckProcess(pinned,pid);
        int[] ids={2301,2302,2303,2304,2310,2318,2320,2333,2334,2338,2353};
        var proof=new PendingProof(); var frozen=new List<string>();
        IntPtr query=ProxyUiSmoke.GetDlgItem(parent,2320);
        for(int attempt=1;attempt<=20;attempt++) {
            proof.Attempts=attempt;
            foreach(var process in System.Diagnostics.Process.GetProcessesByName("ai-relay")) {
                using(process) { if(process.Id!=pid) throw new Exception("BLOCKED: other ai-relay appeared during pending probe; left untouched"); }
            }
            StatusBarNative.CheckProcess(pinned,pid);
            ProxyUiSmoke.CheckOwner(query,pid);
            if(!ProxyUiSmoke.IsWindowEnabled(query)) throw new Exception("Query already disabled before pending probe");
            Message(query,pid,0xF5,0,0); // Genuine native button BM_CLICK.
            if(!ProxyUiSmoke.IsWindowEnabled(query)) {
                bool drained=false;
                foreach(int id in ids) {
                    IntPtr h=ProxyUiSmoke.GetDlgItem(parent,id); ProxyUiSmoke.CheckOwner(h,pid);
                    if(ProxyUiSmoke.IsWindowEnabled(h)) {
                        // A real query can finish while inspecting its controls.
                        // Retry that race; fail only if the query is still frozen.
                        if(ProxyUiSmoke.IsWindowEnabled(query)) { drained=true; break; }
                        throw new Exception("Pending filter not frozen: ID "+id);
                    }
                    frozen.Add(id.ToString());
                }
                if(!drained && !ProxyUiSmoke.IsWindowEnabled(query)) {
                    IntPtr view=ProxyUiSmoke.GetDlgItem(parent,2300); ProxyUiSmoke.CheckOwner(view,pid);
                    if(!ProxyUiSmoke.IsWindowEnabled(view)) throw new Exception("Session/history must remain enabled while pending");
                    proof.Date=(int)Message(ProxyUiSmoke.GetDlgItem(parent,2301),pid,0x147,0,0);
                    proof.Zone=(int)Message(ProxyUiSmoke.GetDlgItem(parent,2333),pid,0x147,0,0);
                    proof.Trend=(int)Message(ProxyUiSmoke.GetDlgItem(parent,2334),pid,0x147,0,0);
                    proof.Table=(int)Message(ProxyUiSmoke.GetDlgItem(parent,2338),pid,0x147,0,0);
                    proof.Export=(int)Message(ProxyUiSmoke.GetDlgItem(parent,2353),pid,0x147,0,0);
                    proof.Seen=true; proof.Frozen=frozen.ToArray(); return proof;
                }
                frozen.Clear();
            }
            // Do not slow/lock the database or alter timers to manufacture pending.
            var timer=System.Diagnostics.Stopwatch.StartNew();
            while(!ProxyUiSmoke.IsWindowEnabled(query) && timer.ElapsedMilliseconds<5000) System.Threading.Thread.Sleep(1);
            if(!ProxyUiSmoke.IsWindowEnabled(query)) throw new Exception("Pending probe drain exceeded five seconds");
        }
        throw new Exception("Pending window not observed in 20 bounded attempts; freeze NOT validated");
    }
    public static Shot Capture(IntPtr window,uint pid,string path,bool clientOnly,StatusBarNative.Rect region) {
        ProxyUiSmoke.CheckOwner(window,pid);
        var work=Task.Run(delegate {
            var previous=StatusBarNative.EnterDpi();
            try {
                var r=clientOnly?StatusBarNative.Client(window,pid):StatusBarNative.Outer(window,pid);
                if(r.Width<1 || r.Height<1 || r.Width>8192 || r.Height>8192) throw new Exception("Invalid physical PrintWindow size");
                using(var bitmap=new Bitmap(r.Width,r.Height,PixelFormat.Format32bppArgb)) {
                    using(var graphics=Graphics.FromImage(bitmap)) {
                        graphics.Clear(Color.Magenta); IntPtr dc=graphics.GetHdc();
                        try { ProxyUiSmoke.CheckOwner(window,pid); if(!PrintWindow(window,dc,clientOnly?3u:2u)) throw new Exception("PrintWindow failed"); }
                        finally { graphics.ReleaseHdc(dc); }
                    }
                    if(region.Width==0 && region.Height==0) region=new StatusBarNative.Rect { Right=r.Width,Bottom=r.Height };
                    if(region.Left<0 || region.Top<0 || region.Right>r.Width || region.Bottom>r.Height || region.Width<=0 || region.Height<=0)
                        throw new Exception("Screenshot inspection region out of bounds");
                    double sum=0; int count=0,bright=0,untouched=0,tiles=0,brightTiles=0;
                    var colors=new HashSet<int>();
                    for(int y=region.Top;y<region.Bottom;y+=2) for(int x=region.Left;x<region.Right;x+=2) {
                        Color c=bitmap.GetPixel(x,y); colors.Add(c.ToArgb()); count++;
                        sum+=0.2126*c.R+0.7152*c.G+0.0722*c.B;
                        if(c.R>=220 && c.G>=220 && c.B>=220) bright++;
                        if(c.R==255 && c.G==0 && c.B==255) untouched++;
                    }
                    // Uniform bright blocks catch unpainted white table/header/arrow
                    // backgrounds without mistaking small glyphs for a white surface.
                    for(int y=region.Top;y+12<=region.Bottom;y+=12) for(int x=region.Left;x+12<=region.Right;x+=12) {
                        int b=0,n=0;
                        for(int dy=0;dy<12;dy+=2) for(int dx=0;dx<12;dx+=2) {
                            var c=bitmap.GetPixel(x+dx,y+dy); n++; if(c.R>=220 && c.G>=220 && c.B>=220) b++;
                        }
                        tiles++; if(b>=n*0.8) brightTiles++;
                    }
                    bitmap.Save(path,ImageFormat.Png); // Preserve evidence even on color failure.
                    if(untouched>count/100 || colors.Count<2) throw new Exception("Blank/incomplete PrintWindow: "+path);
                    var after=clientOnly?StatusBarNative.Client(window,pid):StatusBarNative.Outer(window,pid);
                    if(after.Width!=r.Width || after.Height!=r.Height) throw new Exception("Window resized during physical capture: "+path);
                    return new Shot { Path=path,Width=r.Width,Height=r.Height,Colors=colors.Count,MeanLuminance=sum/count,
                        BrightFraction=(double)bright/count,BrightTileFraction=tiles==0?0:(double)brightTiles/tiles };
                }
            } finally { StatusBarNative.RestoreDpi(previous); }
        });
        if(!work.Wait(8000)) throw new Exception("PrintWindow exceeded eight seconds: "+path);
        return work.GetAwaiter().GetResult();
    }
}
'@)
$joined = $declarations -join "`n"
$usingPattern = '(?m)^using [^;\r\n]+;\r?$'
$usings = @([regex]::Matches($joined,$usingPattern) | ForEach-Object { $_.Value.Trim() } | Select-Object -Unique)
$combined = ($usings -join "`n") + "`n" + [regex]::Replace($joined,$usingPattern,'')
Add-Type -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll,Accessibility.dll -TypeDefinition $combined
if ($ValidateOnly) {
    Write-Output 'PASS: PowerShell entry/declaration extraction and combined C# compilation only; no EXE, GUI, mock, or acceptance tests run'
    return
}

$session = [Diagnostics.Process]::GetCurrentProcess().SessionId
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$mutexName = "Local\AIRelay.$sid.$session.v1"
$child=$null; $childHandle=[IntPtr]::Zero; $childStart=$null; $ownedPid=0
$window=[IntPtr]::Zero; $mock=$null; $temp=$null; $failure=$null
$checks=[Collections.Generic.List[object]]::new()
$shots=[Collections.Generic.List[object]]::new()
$uiFailures=[Collections.Generic.List[string]]::new()
$cleanupErrors=[Collections.Generic.List[string]]::new()
$overall=[Diagnostics.Stopwatch]::StartNew()
$stage='preflight'; $previousDpi=[IntPtr]::Zero
function Assert-NoInstance {
    $existing=$null
    try { $existing=[Threading.Mutex]::OpenExisting($mutexName) }
    catch [Threading.WaitHandleCannotBeOpenedException] { }
    if ($null -ne $existing) { $existing.Dispose(); throw 'BLOCKED: existing same-user/session main instance; no launch or IPC sent' }
    Assert-NoOtherProcess
}
function Assert-NoOtherProcess {
    $others=@(Get-Process -Name 'ai-relay' -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $ownedPid })
    if ($others.Count -gt 0) { throw "BLOCKED: other ai-relay PID(s) $($others.Id -join ','); no input/IPC/kill sent to them" }
}
function Assert-Identity {
    if ($null -eq $child -or $child.HasExited) { throw 'Owned child missing/exited; refusing automation/termination' }
    [StatusBarNative]::CheckProcess($childHandle,[uint32]$ownedPid)
    if ($child.Id -ne $ownedPid -or $child.Handle -ne $childHandle -or $child.StartTime -ne $childStart -or $child.Path -ine $exe) {
        throw 'Pinned child PID/handle/start time/executable identity changed; refusing automation/termination'
    }
}
function Assert-Child {
    Assert-NoOtherProcess
    Assert-Identity
    if ($overall.Elapsed.TotalSeconds -ge 300) { throw "Overall 300-second test budget exceeded at $stage" }
}
function Record([string]$Name,[object]$Value) {
    $checks.Add([pscustomobject]@{ Check=$Name; Value=$Value; Seconds=[Math]::Round($overall.Elapsed.TotalSeconds,2) })
    # Diagnostics must not join function return values (notably Shot objects).
    Write-Host ("CHECK {0}: {1}" -f $Name,($Value | ConvertTo-Json -Compress -Depth 6))
}
function Control([int]$Id) {
    Assert-Child
    [ProxyUiSmoke]::CheckOwner($window,[uint32]$ownedPid)
    $h=[ProxyUiSmoke]::GetDlgItem($window,$Id)
    [ProxyUiSmoke]::CheckOwner($h,[uint32]$ownedPid)
    return $h
}
function Message([int]$Id,[uint32]$Code,[uint32]$W=0,[int]$L=0) {
    [ModernNative]::Message((Control $Id),[uint32]$ownedPid,$Code,$W,$L)
}
function Text([int]$Id) { [ClarityNative]::Text((Control $Id),[uint32]$ownedPid) }
function Visible([int]$Id) { [ProxyUiSmoke]::IsWindowVisible((Control $Id)) }
function Wait-State([string]$Name,[scriptblock]$Test) {
    $timer=[Diagnostics.Stopwatch]::StartNew()
    while ($timer.Elapsed.TotalSeconds -lt 15) {
        Assert-Child
        if (& $Test) { return }
        Start-Sleep -Milliseconds 40
    }
    throw "Bounded wait expired at $stage / $Name"
}
function Click([int]$Id) { [ModernNative]::Click((Control $Id),[uint32]$ownedPid) }
function Select-Segment([int]$Id,[int]$Index,[switch]$MainTab) {
    $tab=Control $Id
    if ([ProxyUiSmoke]::Class($tab) -ne 'SysTabControl32') { throw "ID $Id is not native segmented SysTabControl32" }
    [ModernNative]::ClickTab($childHandle,[uint32]$ownedPid,$tab,$Index)
    Wait-State "native Tab $Id selection $Index" { (Message $Id 0x130B) -eq $Index }
    if (-not $MainTab -and (Message $Id 0x147) -ne $Index) { throw "ID $Id CB_GETCURSEL differs from TCM_GETCURSEL" }
}
function Select-Page([int]$Index,[int]$Expected) {
    Select-Segment 2100 $Index -MainTab
    Wait-State "page $Index control $Expected" { Visible $Expected }
}
function Select-Combo([int]$Id,[int]$Index) {
    $combo=Control $Id
    if ([ProxyUiSmoke]::Class($combo) -ne 'ComboBox') { throw "ID $Id expected real ComboBox" }
    if ((Message $Id 0x14E ([uint32]$Index)) -ne $Index) { throw "Combo $Id CB_SETCURSEL failed" }
    # Only conventional ComboBoxes need the older script-compatible command path.
    [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,[uint32]($Id -bor 0x10000))
}
function Snapshot([string]$Name,[IntPtr]$Handle,[switch]$Outer,[object]$Region=$null) {
    Assert-Child
    if (-not [ProxyUiSmoke]::IsWindowVisible($Handle)) { throw "Screenshot target hidden: $Name" }
    $rect=New-Object StatusBarNative+Rect
    if ($null -ne $Region) { $rect=$Region }
    $shot=[ModernNative]::Capture($Handle,[uint32]$ownedPid,(Join-Path $output "$Name.png"),(-not $Outer),$rect)
    $shots.Add([pscustomobject]@{ Name=$Name; DPI=[ClarityNative]::GetDpiForWindow($Handle); Shot=$shot })
    Record "physical PrintWindow $Name" $shot
    return $shot
}
function Capture-Segment([int]$Id,[int]$Index,[string]$Prefix='dark') {
    Snapshot "$Prefix-segment-$Id-$Index" (Control $Id) | Out-Null
}
function Assert-Dark([object]$Shot,[string]$Name,[double]$MaxBright=0.12) {
    if ($Shot.MeanLuminance -gt 125 -or $Shot.BrightFraction -gt $MaxBright -or $Shot.BrightTileFraction -gt 0.02) {
        throw "Dark surface has a large light/white background: $Name; luminance=$($Shot.MeanLuminance), bright=$($Shot.BrightFraction), brightTiles=$($Shot.BrightTileFraction), PNG=$($Shot.Path)"
    }
}
function Assert-HistoryLayout([string]$Label) {
    Assert-Child
    $rects=[Collections.Generic.List[object]]::new()
    $client=[StatusBarNative]::Client($window,[uint32]$ownedPid)
    foreach ($h in [ModernNative]::DirectChildren($window,[uint32]$ownedPid)) {
        if (-not [ProxyUiSmoke]::IsWindowVisible($h)) { continue }
        $id=[ClarityNative]::GetDlgCtrlID($h); $kind=[ProxyUiSmoke]::Class($h)
        $r=[ModernNative]::Bounds($h,$window,[uint32]$ownedPid)
        if ($r.Width -le 0 -or $r.Height -le 0) { continue }
        $entry=[pscustomobject]@{ ID=$id; Class=$kind; Rect="$r"; Bounds=$r }
        $rects.Add($entry)
        if ($r.Left -lt 0 -or $r.Top -lt 0 -or $r.Right -gt $client.Width -or $r.Bottom -gt $client.Height) {
            $uiFailures.Add("$Label clipped visible control ID=$id class=$kind rect=$r client=$client")
        }
    }
    for ($a=0;$a -lt $rects.Count;$a++) {
        for ($b=$a+1;$b -lt $rects.Count;$b++) {
            $ra=$rects[$a].Bounds; $rb=$rects[$b].Bounds
            if ($ra.Left -lt $rb.Right -and $ra.Right -gt $rb.Left -and $ra.Top -lt $rb.Bottom -and $ra.Bottom -gt $rb.Top) {
                $uiFailures.Add("$Label overlap: ID=$($rects[$a].ID) $($rects[$a].Class) [$ra] vs ID=$($rects[$b].ID) $($rects[$b].Class) [$rb]")
            }
        }
    }
    Record "$Label all visible direct controls (closed combo bounds)" @($rects.ToArray())
    # Continue collecting screenshots, but emit exact conflicts immediately too.
    foreach ($problem in $uiFailures) { Write-Warning $problem }
}
function Assert-Report([string]$Expected='Local/Day') {
    Wait-State "backend 10 requests, $Expected" {
        [ProxyUiSmoke]::IsWindowEnabled((Control 2320)) -and
        (Text 2325).Contains('匹配 10 请求') -and (Text 2325).Contains($Expected)
    }
    foreach ($needle in @('请求 10；完整 10 / 部分 0 / 未知 0','输入 100 / 输出 50 / 总 150','缓存命中 30','推理 20')) {
        if (-not (Text 2330).Contains($needle)) { throw "Real backend summary missing '$needle': $(Text 2330)" }
    }
    $rows=@((Text 2331) -split '\r?\n' | Where-Object { $_.Contains('local-usage-model') })
    if ($rows.Count -ne 10) { throw "Metadata expected 10 real rows, got $($rows.Count)" }
    foreach ($row in $rows) {
        foreach ($needle in @('127.0.0.1','10/5/15','Success','Complete')) {
            if (-not $row.Contains($needle)) { throw "Real metadata row missing $needle : $row" }
        }
    }
}
function Assert-ConfigUnchanged {
    $now=[Convert]::ToBase64String([IO.File]::ReadAllBytes($config))
    if ($now -cne $configBytes) { throw 'Theme/draft/switch test wrote config.toml without Save' }
}

# Fail before sockets, temp config, launch or IPC when any other process exists.
Assert-NoInstance
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
    throw "Build unavailable: $([IO.Path]::GetFullPath($ExePath)). No child launched; build later and rerun. This script never runs Cargo."
}
$exe=(Get-Item -LiteralPath $ExePath).FullName
$outputRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\target\modern-ui-screenshots'))
$output=Join-Path $outputRoot ([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')+'-'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($output)
try {
    $previousDpi=[StatusBarNative]::EnterDpi()
    $stage='private loopback fixture'
    Assert-NoInstance
    $temp=Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-modern-ui-'+[Guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($temp)
    $config=Join-Path $temp 'config.toml'
    $mock=[ProxyUiMock]::new('modern-ui-local-only')
    $port=[ProxyUiSmoke]::FreePort()
    if ($port -eq $mock.Port) { throw 'Ephemeral mock/proxy ports collided; safely rerun' }
    $toml="schema_version = 1`n[upstream]`nurl = 'http://127.0.0.1:$($mock.Port)/v1'`n[listen]`naddress = '127.0.0.1'`nport = $port`n[proxy]`nauto_start = false`nforward_enabled = false`nconnect_enabled = false`n[ui]`ntheme = 'dark'`nremember_window_geometry = false`n[system]`nlogin_auto_start = false`n"
    [IO.File]::WriteAllText($config,$toml,[Text.UTF8Encoding]::new($false))
    $stage='launch and pin exact child'
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
    Wait-State 'own main HWND' { $script:window=[ProxyUiSmoke]::Find([uint32]$ownedPid); $window -ne [IntPtr]::Zero }
    Wait-State 'saved config loaded and stopped' {
        $h=[ProxyUiSmoke]::GetDlgItem($window,2002)
        $h -ne [IntPtr]::Zero -and [ProxyUiSmoke]::IsWindowEnabled($h) -and
        -not [ProxyUiSmoke]::IsWindowEnabled((Control 2003))
    }
    $configBytes=[Convert]::ToBase64String([IO.File]::ReadAllBytes($config))
    Record 'pinned child, stopped private config, loopback only' @{ PID=$ownedPid; Start=$childStart.ToString('o'); Exe=$exe; DPI=[ClarityNative]::GetDpiForWindow($window) }
    [StatusBarNative]::Resize($window,[uint32]$ownedPid,$false) | Out-Null
    Start-Sleep -Milliseconds 200
    $stage='ten local mock requests'
    Click 2102
    Wait-State 'proxy running' { [ProxyUiSmoke]::IsWindowEnabled((Control 2003)) }
    for ($i=0;$i -lt 10;$i++) {
        Assert-Child
        [ProxyUiSmoke]::RequestBody($port,'/usage-json',[ProxyUiSmoke]::UsageJson)
    }
    if ($mock.Requests -ne 10 -or $mock.ProxyCredentialsReceived -ne 0) { throw 'Mock request count/credential isolation mismatch' }
    Click 2103
    Wait-State 'proxy stopped after mock seed' { [ProxyUiSmoke]::IsWindowEnabled((Control 2002)) -and -not [ProxyUiSmoke]::IsWindowEnabled((Control 2003)) }
    if (-not [ProxyUiSmoke]::PortClosed($port)) { throw 'Private proxy still listening after stop' }
    Record 'exactly 10 loopback mock responses, 100/50/150 tokens; listener stopped' $mock.Requests

    $stage='native segmented identity and compatibility'
    Select-Page 1 2300
    if ((Message 2333 0x147) -ne 1) { throw 'Initial timezone must follow system Local' }
    Wait-State 'history scope query enabled' { [ProxyUiSmoke]::IsWindowEnabled((Control 2320)) }
    foreach ($id in @(2300,2301,2333,2334,2338,2353,2030)) {
        $h=Control $id
        if ([ProxyUiSmoke]::Class($h) -ne 'SysTabControl32') { throw "ID $id must now be SysTabControl32, actual=$([ProxyUiSmoke]::Class($h))" }
        $old=Message $id 0x147
        $count=Message $id 0x146
        $other=($old+1)%$count
        if ((Message $id 0x14E ([uint32]$other)) -ne $other -or (Message $id 0x147) -ne $other -or (Message $id 0x130B) -ne $other) { throw "CB_GET/SETCURSEL compatibility broken: ID $id" }
        if ((Message $id 0x14E ([uint32]$old)) -ne $old) { throw "CB_SETCURSEL restore failed: ID $id" }
        Record "native segmented $id and silent CB_GET/SETCURSEL compatibility" @{ Count=$count; Restored=$old }
    }
    Click 2320
    Assert-Report
    if ((Message 2337 0x1004) -ne 10) { throw 'Native detail table 2337 expected 10 rows' }
    Snapshot 'history-default' $window -Outer | Out-Null
    Assert-HistoryLayout 'history-default'
    Capture-Segment 2300 0

    $stage='2300 genuine native mouse notification: session/history'
    Select-Segment 2300 1
    Wait-State 'session 2020 visible, history table hidden' { (Visible 2020) -and -not (Visible 2337) -and -not (Visible 2301) }
    Wait-State 'session contains real 10-request usage' { (Text 2020).Contains('150') }
    Capture-Segment 2300 1
    Snapshot 'history-session' $window -Outer | Out-Null
    Select-Segment 2300 0
    Wait-State 'history restored without session overlap' { (Visible 2337) -and -not (Visible 2020) -and (Message 2337 0x1004) -eq 10 }

    $stage='2301 date preset native notification and custom visibility'
    foreach ($index in @(1,2,3,4,0)) {
        Select-Segment 2301 $index
        Wait-State "date $index only custom shows 2302/2303" { (Visible 2302) -eq ($index -eq 4) -and (Visible 2303) -eq ($index -eq 4) }
        Capture-Segment 2301 $index
        if ($index -eq 4) {
            $customStart=[DateTime]::Now.ToString('yyyy-MM-dd')
            $customEnd=[DateTime]::Now.AddDays(1).ToString('yyyy-MM-dd')
            [ProxyUiSmoke]::SetText((Control 2302),[uint32]$ownedPid,$customStart)
            [ProxyUiSmoke]::SetText((Control 2303),[uint32]$ownedPid,$customEnd)
            Snapshot 'history-custom-date' $window -Outer | Out-Null
            Assert-HistoryLayout 'history-custom-date'

            $stage='custom date RESET hides inputs and retains applied report'
            $appliedSummary=Text 2330
            $appliedMetadata=Text 2331
            $appliedRows=Message 2337 0x1004
            Click 2317
            Wait-State 'RESET date preset 0, both date inputs hidden, reset notification handled' {
                (Message 2301 0x147) -eq 0 -and (Message 2333 0x147) -eq 1 -and -not (Visible 2302) -and -not (Visible 2303) -and
                (Text 2325).Contains('筛选草稿已重置')
            }
            if ((Text 2330) -cne $appliedSummary -or (Text 2331) -cne $appliedMetadata -or
                (Message 2337 0x1004) -ne $appliedRows -or -not (Visible 2337) -or -not (Visible 2332)) {
                throw 'RESET replaced/cleared the previously applied summary, metadata, table rows or chart visibility'
            }
            Record 'custom date RESET: CB preset 0, 2302/2303 hidden, old applied report retained' @{ Preset=0; Rows=$appliedRows }
            $stage='2301 date preset native notification and custom visibility'
        }
    }

    $stage='2333/2334 genuine clicks trigger real background queries'
    foreach ($zone in @(0,1,0)) {
        Select-Segment 2333 $zone
        $zoneName=if ($zone -eq 1) { 'Local' } else { 'Utc' }
        Assert-Report "$zoneName/Day"
        Capture-Segment 2333 $zone
    }
    foreach ($grain in @(1,2,0)) {
        Select-Segment 2334 $grain
        $grainName=@('Day','Hour','Month')[$grain]
        Assert-Report "Utc/$grainName"
        if (-not (Text 2330).Contains("趋势 $grainName")) { throw "TREND $grain did not update applied backend query: $(Text 2330)" }
        Capture-Segment 2334 $grain
    }

    $stage='pending freeze and selection preservation'
    Assert-Child
    $pendingDateStart=Text 2302
    $pendingDateEnd=Text 2303
    $pending=[ModernNative]::ProbePending($childHandle,[uint32]$ownedPid,$window)
    if (-not $pending.Seen) { throw 'Pending not observed; cannot claim freeze passed' }
    Assert-Report 'Utc/Day'
    foreach ($pair in @(@(2301,$pending.Date),@(2333,$pending.Zone),@(2334,$pending.Trend),@(2338,$pending.Table),@(2353,$pending.Export))) {
        if ((Message $pair[0] 0x147) -ne $pair[1]) { throw "Pending changed selection ID $($pair[0])" }
    }
    if ((Text 2302) -cne $pendingDateStart -or (Text 2303) -cne $pendingDateEnd) { throw 'Pending reset hidden custom date drafts' }
    Record 'observed real pending, filters frozen, view enabled and drafts preserved' $pending

    $stage='advanced real combo, dark table/header, layout'
    Click 2318
    Wait-State 'advanced combo visible, chart hidden, table retained' { (Visible 2310) -and -not (Visible 2332) -and (Visible 2337) }
    Select-Combo 2314 3
    Click 2320
    Assert-Report 'Utc/Day'
    Snapshot 'dark-advanced-table-window' $window -Outer | Out-Null
    Assert-HistoryLayout 'history-advanced-table'
    $table=Control 2337
    if ([ProxyUiSmoke]::Class($table) -ne 'SysListView32') { throw '2337 is not native table' }
    $tableShot=Snapshot 'dark-advanced-table-2337' $table
    Assert-Dark $tableShot 'native table 2337'
    $header=[IntPtr](Message 2337 0x101F)
    [ProxyUiSmoke]::CheckOwner($header,[uint32]$ownedPid)
    if ([ProxyUiSmoke]::Class($header) -ne 'SysHeader32') { throw '2337 header is not native SysHeader32' }
    $headerShot=Snapshot 'dark-table-header' $header
    Assert-Dark $headerShot 'native header' 0.20
    $combo=Control 2310
    $info=[ModernNative]::Combo($combo,[uint32]$ownedPid)
    if ((Message 2310 0x157) -ne 0) { throw '2310 unexpectedly dropped open' }
    $comboShot=Snapshot 'dark-combo-2310-closed' $combo
    Assert-Dark $comboShot 'closed combo 2310'
    $arrowShot=Snapshot 'dark-combo-2310-arrow-inspection' $combo -Region $info.Button
    # The native light chevron itself occupies about 7% of this small region.
    # Apply the normal dark-surface test (including bright background tiles),
    # rather than treating a correctly rendered light glyph as a white backdrop.
    Assert-Dark $arrowShot 'closed combo 2310 arrow'

    $stage='2338 real detail/group/text/metadata notifications'
    foreach ($index in @(1,2,3,0)) {
        Select-Segment 2338 $index
        Wait-State "table view $index visibility" {
            (Visible 2337) -eq ($index -le 1) -and (Visible 2330) -eq ($index -eq 2) -and (Visible 2331) -eq ($index -eq 3)
        }
        if ($index -le 1) {
            $expected=if ($index -eq 0) { 10 } else { 1 }
            if ((Message 2337 0x1004) -ne $expected) { throw "2338 view $index rows mismatch: expected $expected" }
            $head=[IntPtr](Message 2337 0x101F)
            $columns=[ModernNative]::Message($head,[uint32]$ownedPid,0x1200,0,0)
            if ($columns -ne @(14,9)[$index]) { throw "2338 view $index native header column count mismatch: $columns" }
        }
        Capture-Segment 2338 $index
        Snapshot "dark-table-view-$index" $window -Outer | Out-Null
        Assert-HistoryLayout "history-table-view-$index"
    }
    $stage='2353 export scope native mouse selection (no file export/dialog)'
    foreach ($index in @(1,0)) {
        Select-Segment 2353 $index
        Capture-Segment 2353 $index
    }
    Click 2318
    Wait-State 'default chart layout restored' { (Visible 2332) -and -not (Visible 2310) }
    Snapshot 'history-dark' $window -Outer | Out-Null
    $darkChart=Snapshot 'history-dark-chart' (Control 2332)
    Assert-Dark $darkChart 'history chart'

    $stage='safe native checkbox switch: logs 2228, never login 2031'
    Select-Page 2 2228
    if (([ProxyUiSmoke]::GetWindowLongW((Control 2228),-16) -band 15) -ne 3) { throw '2228 must retain BS_AUTOCHECKBOX' }
    if ((Message 2228 0xF0) -ne 0) { throw 'Expected initial logs errors-only BM_GETCHECK=0' }
    Snapshot 'dark-switch-2228-off' (Control 2228) | Out-Null
    Message 2228 0xF5 | Out-Null
    Wait-State 'BM_CLICK checks native switch' { (Message 2228 0xF0) -eq 1 }
    Snapshot 'dark-switch-2228-on' (Control 2228) | Out-Null
    Click 2228
    Wait-State 'native mouse unchecks switch' { (Message 2228 0xF0) -eq 0 }
    # The mouse click focused this native Button. Deliver normal key messages to
    # that HWND, not global SendKeys; no input can escape to an unrelated window.
    Message 2228 0x100 0x20 1 | Out-Null
    Message 2228 0x101 0x20 ([int]-1073741823) | Out-Null
    Wait-State 'native Space checks switch' { (Message 2228 0xF0) -eq 1 }
    Message 2228 0xF5 | Out-Null
    Wait-State 'switch restored off' { (Message 2228 0xF0) -eq 0 }
    Record 'switch BM_GETCHECK 0/1, BM_CLICK, mouse and Space; 2031 never clicked' $true

    $stage='settings draft and 2030 native theme preview without saving'
    Select-Page 3 2039
    Select-Combo 2039 0
    Wait-State 'upstream category visible' {
        @([ModernNative]::DirectChildren($window,[uint32]$ownedPid) | Where-Object {
            [ProxyUiSmoke]::Class($_) -eq 'Edit' -and [ProxyUiSmoke]::IsWindowVisible($_) -and
            ([ProxyUiSmoke]::GetWindowLongW($_,-16) -band 0x20) -eq 0 -and
            [ClarityNative]::Text($_,[uint32]$ownedPid) -eq "http://127.0.0.1:$($mock.Port)/v1"
        }).Count -eq 1
    }
    $draftEdit=@([ModernNative]::DirectChildren($window,[uint32]$ownedPid) | Where-Object {
        [ProxyUiSmoke]::Class($_) -eq 'Edit' -and [ProxyUiSmoke]::IsWindowVisible($_) -and
        ([ProxyUiSmoke]::GetWindowLongW($_,-16) -band 0x20) -eq 0 -and
        [ClarityNative]::Text($_,[uint32]$ownedPid) -eq "http://127.0.0.1:$($mock.Port)/v1"
    })[0]
    $draft="http://127.0.0.1:$($mock.Port)/unsaved-modern-ui-draft"
    [ProxyUiSmoke]::SetText($draftEdit,[uint32]$ownedPid,$draft)
    Select-Combo 2039 5
    Wait-State 'appearance segmented 2030 visible' { Visible 2030 }
    foreach ($index in @(0,1,2)) {
        $name=@('system','light','dark')[$index]
        Select-Segment 2030 $index
        Start-Sleep -Milliseconds 200
        Capture-Segment 2030 $index $name
        Assert-ConfigUnchanged
        if ([ClarityNative]::Text($draftEdit,[uint32]$ownedPid) -cne $draft) { throw "Theme $name reset unsaved settings draft" }
        if ($index -eq 0) { continue }
        Select-Page 1 2300
        Wait-State 'history default view restored' { (Visible 2332) -and (Visible 2337) }
        Snapshot "history-$name" $window -Outer | Out-Null
        $statusCandidates=@([ModernNative]::DirectChildren($window,[uint32]$ownedPid) | Where-Object {
            [ProxyUiSmoke]::Class($_) -eq 'Static' -and [ProxyUiSmoke]::IsWindowVisible($_) -and
            [ClarityNative]::Text($_,[uint32]$ownedPid).StartsWith('代理状态：')
        })
        if ($statusCandidates.Count -ne 1) { throw 'Cannot uniquely identify the visible native status bar' }
        $statusShot=Snapshot "$name-status-bar" $statusCandidates[0]
        if ($name -eq 'dark') { Assert-Dark $statusShot 'dark status bar' }
        elseif ($statusShot.MeanLuminance -lt 150) { throw 'Light status bar did not adopt light palette' }
        $chart=Snapshot "history-$name-preview-chart" (Control 2332)
        $tablePreview=Snapshot "$name-table-preview" (Control 2337)
        Assert-HistoryLayout "history-$name-preview"
        if ($name -eq 'light') { $lightChart=$chart; $lightTable=$tablePreview }
        else {
            Assert-Dark $chart 'dark chart after theme round trip'
            Assert-Dark $tablePreview 'dark table after theme round trip'
            if ($lightChart.MeanLuminance -lt 150 -or $lightChart.MeanLuminance-$chart.MeanLuminance -lt 60 -or
                $lightTable.MeanLuminance-$tablePreview.MeanLuminance -lt 60) { throw 'Real light/dark preview surfaces did not change distinctly' }
        }
        Select-Page 2 2228
        Snapshot "$name-switch-2228-off" (Control 2228) | Out-Null
        Message 2228 0xF5 | Out-Null
        Wait-State "$name switch on" { (Message 2228 0xF0) -eq 1 }
        Snapshot "$name-switch-2228-on" (Control 2228) | Out-Null
        Message 2228 0xF5 | Out-Null
        Select-Page 3 2039
        Wait-State 'appearance control returns without draft reset' { Visible 2030 }
        if ((Message 2030 0x147) -ne $index) { throw 'Page navigation reset theme selection' }
        Assert-ConfigUnchanged
    }
    Select-Combo 2039 0
    Wait-State 'draft visible and retained after all theme/page changes' {
        [ProxyUiSmoke]::IsWindowVisible($draftEdit) -and [ClarityNative]::Text($draftEdit,[uint32]$ownedPid) -ceq $draft
    }
    Assert-ConfigUnchanged
    Record 'settings and history drafts preserved; theme preview never saved config' $true
    if ($mock.Requests -ne 10) { throw 'Unexpected extra network requests during theme/history testing' }
    if ($uiFailures.Count -gt 0) { throw ("Visible control layout failures:`n"+($uiFailures -join "`n")) }
    $stage='clean owned-child exit'
    Assert-Child
    [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,1002)
    if (-not $child.WaitForExit(38000)) { throw 'Owned child clean exit exceeded 38 seconds' }
    if ($child.ExitCode -ne 0) { throw "Owned child exit code $($child.ExitCode)" }
    Assert-ConfigUnchanged
    Record 'clean exact-child exit, unsaved config bytes unchanged' $ownedPid
} catch {
    $failure=$_
    Write-Warning ("FAIL stage={0}; {1}; {2}" -f $stage,$_.Exception.Message,$_.InvocationInfo.PositionMessage)
} finally {
    $exited=$null -eq $child
    if ($null -ne $child) {
        try {
            if (-not $child.HasExited) {
                # Cleanup deliberately omits the other-instance guard: even if a
                # foreign instance appeared, ONLY our already-pinned child exits.
                Assert-Identity
                if ($window -eq [IntPtr]::Zero) { $window=[ProxyUiSmoke]::Find([uint32]$ownedPid) }
                if ($window -ne [IntPtr]::Zero) { [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,1002) }
                if (-not $child.WaitForExit(38000)) {
                    Assert-Identity
                    $child.Kill() # Process object retains the original OS handle.
                    if (-not $child.WaitForExit(5000)) { throw 'Pinned child termination exceeded five seconds' }
                    $cleanupErrors.Add('Owned child required forced termination after 38 seconds')
                }
            }
            $exited=$child.HasExited
        } catch { $cleanupErrors.Add($_.Exception.Message) }
        finally { $child.Dispose(); $child=$null }
    }
    if ($null -ne $mock) {
        try { $mock.Dispose() } catch { $cleanupErrors.Add("Bounded mock cleanup: $($_.Exception.Message)") }
    }
    if ($previousDpi -ne [IntPtr]::Zero) { [StatusBarNative]::RestoreDpi($previousDpi) }
    if ($null -ne $temp -and $exited) {
        for ($attempt=0;$attempt -lt 10;$attempt++) {
            try { [IO.Directory]::Delete($temp,$true); break }
            catch {
                if ($attempt -eq 9) { $cleanupErrors.Add("Temp cleanup failed, retained $temp : $($_.Exception.Message)") }
                else { Start-Sleep -Milliseconds 150 }
            }
        }
    } elseif ($null -ne $temp) { $cleanupErrors.Add("Child exit unconfirmed; temp retained at $temp") }
    $detail=$null
    if ($null -ne $failure) {
        $detail=[pscustomobject]@{ Stage=$stage; Message=$failure.Exception.Message; Position=$failure.InvocationInfo.PositionMessage; Stack=$failure.ScriptStackTrace; Error=$failure.ToString() }
    }
    $report=[pscustomobject]@{
        Passed=($null -eq $failure -and $cleanupErrors.Count -eq 0); Exe=$exe; PID=$ownedPid;
        Failure=$detail; LayoutFailures=@($uiFailures.ToArray()); CleanupErrors=@($cleanupErrors.ToArray());
        Checks=@($checks.ToArray()); Screenshots=@($shots.ToArray()); Seconds=[Math]::Round($overall.Elapsed.TotalSeconds,2);
        Limits='Current monitor physical DPI only; no multi-monitor, high-contrast, screen-reader or physical tray acceptance. No Cargo, real upstream/key, Startup mutation, config Save, global input or IPC to another instance. Pending is observed, never artificially delayed.'
    }
    [IO.File]::WriteAllText((Join-Path $output 'results.json'),($report | ConvertTo-Json -Depth 12),[Text.UTF8Encoding]::new($true))
    Write-Output "Modern UI artifacts: $output"
}
if ($null -ne $failure) { throw $failure }
if ($cleanupErrors.Count -gt 0) { throw ($cleanupErrors -join '; ') }
Write-Output 'PASS: native segmented clicks/compatibility, real queries/pending, dark table/header/combo, native switches, theme/draft preservation, history layout, physical screenshots and owned-child cleanup'
