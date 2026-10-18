# Windows PowerShell 5.1 x64；交互式 Windows 10/11 桌面。
# 仅本脚本固定句柄的子进程；不构建、不联网、不启动代理、不保存设置。
# 先运行：powershell.exe -NoProfile -File tools\smoke-rounded-ui.ps1 -ValidateOnly
# 收到构建完成通知后再去掉 -ValidateOnly；不会执行其他 smoke 脚本的 body。
# 高对比度移除 region 由 rounded/theme 的隔离测试负责；本脚本不切换系统设置。
[CmdletBinding()]
param(
    [string]$ExePath,
    [switch]$ValidateOnly
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([IntPtr]::Size -ne 8) { throw 'Use a fresh 64-bit powershell.exe -NoProfile' }
if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $PSScriptRoot '..\target\rounded-ui-validation\target\debug\ai-relay.exe'
}

# 只提取 C# 声明，合并编译以兼容 Windows PowerShell 5.1 内存程序集。
# 被提取的 ProxyUiMock 只是类型声明，绝不实例化或调用其网络方法。
$parts = @(
    @{ File='smoke-proxy-ui.ps1'; Pattern="(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@" },
    @{ File='smoke-ui-clarity.ps1'; Pattern="(?s)Add-Type -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll -TypeDefinition @'\r?\n(.*?)\r?\n'@" },
    @{ File='smoke-status-bar-ui.ps1'; Pattern='(?s)\$statusDeclaration = @''\r?\n(.*?)\r?\n''@' },
    @{ File='smoke-modern-ui.ps1'; Pattern='(?s)\$declarations\.Add\(@''\r?\n(.*?)\r?\n''@\)' },
    @{ File='smoke-date-picker-ui.ps1'; Pattern='(?s)\$declarations\.Add\(@''\r?\n(.*?)\r?\n''@\)' }
)
foreach ($name in @('ProxyUiSmoke','ProxyUiMock','ClarityNative','StatusBarNative','ModernNative','DatePickerNative','RoundedNative')) {
    if ($name -as [type]) { throw "Helper $name already loaded; use a fresh powershell.exe -NoProfile" }
}
$declarations = [Collections.Generic.List[string]]::new()
foreach ($part in $parts) {
    $source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot $part.File))
    $found = [regex]::Matches($source,$part.Pattern)
    if ($found.Count -ne 1) { throw "Expected one declaration in $($part.File); no older script executed" }
    $declarations.Add($found[0].Groups[1].Value)
}
$declarations.Add(@'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
public static class RoundedNative {
    [DllImport("user32.dll")] static extern int GetWindowRgn(IntPtr window,IntPtr region);
    [DllImport("gdi32.dll")] static extern IntPtr CreateRectRgn(int l,int t,int r,int b);
    [DllImport("gdi32.dll")] static extern IntPtr CreateRoundRectRgn(int l,int t,int r,int b,int w,int h);
    [DllImport("gdi32.dll")] static extern int GetRgnBox(IntPtr region,out StatusBarNative.Rect box);
    [DllImport("gdi32.dll")] static extern bool PtInRegion(IntPtr region,int x,int y);
    [DllImport("gdi32.dll")] static extern bool EqualRgn(IntPtr a,IntPtr b);
    [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr obj);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr h,uint m,UIntPtr w,StringBuilder l,uint flags,uint timeout,out UIntPtr result);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
    [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint thread,ref GuiInfo info);
    [StructLayout(LayoutKind.Sequential)] struct GuiInfo {
        public uint Size,Flags;
        public IntPtr Active,Focus,Capture,MenuOwner,MoveSize,Caret;
        public StatusBarNative.Rect CaretRect;
    }
    public sealed class RegionProof {
        public string Class,Bounds;
        public int Width,Height,Diameter;
        public uint DPI;
        public bool CornersOutside,MiddleInside,ExactSixDipRadius;
    }
    static void Guard(IntPtr pinned,uint pid,IntPtr h) {
        StatusBarNative.CheckProcess(pinned,pid); ProxyUiSmoke.CheckOwner(h,pid);
    }
    public static void NoOtherProcess(IntPtr pinned,uint pid) {
        StatusBarNative.CheckProcess(pinned,pid);
        foreach(var p in Process.GetProcessesByName("ai-relay")) using(p) {
            if(p.Id!=pid) throw new Exception("BLOCKED: another ai-relay appeared; no input/IPC/kill sent to it");
        }
    }
    public static RegionProof Region(IntPtr pinned,uint pid,IntPtr h) {
        Guard(pinned,pid,h);
        var previous=StatusBarNative.EnterDpi();
        IntPtr copy=IntPtr.Zero,expected=IntPtr.Zero;
        try {
            var outer=StatusBarNative.Outer(h,pid);
            uint dpi=ClarityNative.GetDpiForWindow(h);
            if(dpi<96 || outer.Width<4 || outer.Height<4) throw new Exception("Invalid physical native geometry");
            int diameter=Math.Min(Math.Min(outer.Width,outer.Height),(int)((12L*Math.Min(768u,dpi)+48)/96));
            copy=CreateRectRgn(0,0,0,0);
            if(copy==IntPtr.Zero) throw new Exception("Cannot allocate local HRGN");
            // GetWindowRgn copies a foreign HWND's region into OUR local GDI object.
            // GetRgnBox/PtInRegion/EqualRgn then use ONLY that local HRGN. No remote
            // pointer/message or target-process allocation is involved in this check.
            Guard(pinned,pid,h);
            if(GetWindowRgn(h,copy)!=3) throw new Exception("Missing/ non-complex rounded window region; build rounded.rs first");
            StatusBarNative.Rect box;
            if(GetRgnBox(copy,out box)==0 || box.Left!=0 || box.Top!=0 || box.Right!=outer.Width || box.Bottom!=outer.Height)
                throw new Exception("Region bounds != FULL native GetWindowRect: "+box+" / "+outer);
            bool outside=!PtInRegion(copy,0,0) && !PtInRegion(copy,outer.Width-1,0) &&
                !PtInRegion(copy,0,outer.Height-1) && !PtInRegion(copy,outer.Width-1,outer.Height-1);
            bool inside=PtInRegion(copy,outer.Width/2,outer.Height/2) && PtInRegion(copy,outer.Width/2,0) &&
                PtInRegion(copy,0,outer.Height/2) && PtInRegion(copy,outer.Width-1,outer.Height/2) &&
                PtInRegion(copy,outer.Width/2,outer.Height-1);
            // Match the public GDI rasterization, including its exclusive last row/column.
            expected=CreateRoundRectRgn(0,0,outer.Width+1,outer.Height+1,diameter,diameter);
            if(expected==IntPtr.Zero || !outside || !inside || !EqualRgn(copy,expected))
                throw new Exception("Wrong rounded region: corners/middle/6 DIP radius mismatch");
            var after=StatusBarNative.Outer(h,pid);
            if(after.Width!=outer.Width || after.Height!=outer.Height) throw new Exception("Control resized during region check");
            return new RegionProof { Class=ProxyUiSmoke.Class(h),Bounds=box.ToString(),Width=outer.Width,Height=outer.Height,
                Diameter=diameter,DPI=dpi,CornersOutside=outside,MiddleInside=inside,ExactSixDipRadius=true };
        } finally {
            bool ok=true;
            if(expected!=IntPtr.Zero) ok=DeleteObject(expected) && ok;
            if(copy!=IntPtr.Zero) ok=DeleteObject(copy) && ok;
            StatusBarNative.RestoreDpi(previous);
            if(!ok) throw new Exception("Local GDI region release failed");
        }
    }
    public static bool Caret(IntPtr pinned,uint pid,IntPtr edit) {
        Guard(pinned,pid,edit); uint actual;
        uint thread=GetWindowThreadProcessId(edit,out actual);
        var info=new GuiInfo { Size=(uint)Marshal.SizeOf(typeof(GuiInfo)) };
        if(actual!=pid || !GetGUIThreadInfo(thread,ref info)) throw new Exception("Cannot inspect native caret/focus");
        return info.Focus==edit && info.Caret==edit && info.CaretRect.Height>0;
    }
    static string SmallText(IntPtr pinned,uint pid,IntPtr h) {
        Guard(pinned,pid,h); var text=new StringBuilder(512); UIntPtr result;
        // WM_GETTEXT is a system-marshaled message below WM_USER.
        if(SendMessageTimeoutW(h,13,new UIntPtr(512),text,0x22,2000,out result)==IntPtr.Zero)
            throw new Exception("Operation-state WM_GETTEXT timeout");
        return text.ToString();
    }
    static Dictionary<string,ulong> Operation(IntPtr pinned,uint pid,IntPtr parent) {
        var values=new Dictionary<string,ulong>();
        foreach(string part in SmallText(pinned,pid,ProxyUiSmoke.GetDlgItem(parent,2354)).Split(';')) {
            string[] pair=part.Split('='); ulong value;
            if(pair.Length==2 && UInt64.TryParse(pair[1],out value)) values.Add(pair[0],value);
        }
        foreach(string key in new string[]{"idle","sequence","fence","rejected","generation"})
            if(!values.ContainsKey(key)) throw new Exception("Missing current history operation state: "+key);
        return values;
    }
    public sealed class PendingProof {
        public int Attempts;
        public ulong Before,Sequence,Generation;
        public long DisabledButtonState;
        public bool PendingSeen,Completed;
        public string[] Frozen;
        public RegionProof DisabledRegion;
    }
    public static PendingProof Query(IntPtr pinned,uint pid,IntPtr parent) {
        Guard(pinned,pid,parent);
        IntPtr query=ProxyUiSmoke.GetDlgItem(parent,2320);
        for(int attempt=1;attempt<=20;attempt++) {
            NoOtherProcess(pinned,pid); Guard(pinned,pid,query);
            var before=Operation(pinned,pid,parent);
            if(before["idle"]!=1 || before["rejected"]!=0 || !ProxyUiSmoke.IsWindowEnabled(query))
                throw new Exception("Query not idle before native click");
            // Genuine BUTTON procedure -> BN_CLICKED -> real local history worker.
            // Never WM_COMMAND QUERY, fake result, forced EnableWindow or DB lock.
            ModernNative.Message(query,pid,0xF5,0,0);
            var pending=Operation(pinned,pid,parent);
            if(pending["sequence"]!=before["sequence"]+1) throw new Exception("Native BM_CLICK did not submit exactly one query");
            var frozen=new List<string>(); bool seen=pending["idle"]==0;
            long state=0; RegionProof region=null;
            if(seen) {
                foreach(int id in new int[]{2320,2304,2310,2302,2303}) {
                    IntPtr h=ProxyUiSmoke.GetDlgItem(parent,id); Guard(pinned,pid,h);
                    if(ProxyUiSmoke.IsWindowEnabled(h)) {
                        if(Operation(pinned,pid,parent)["idle"]==1) { seen=false; break; }
                        throw new Exception("Actual pending control not disabled: "+id);
                    }
                    frozen.Add(id.ToString());
                }
                if(seen && !ProxyUiSmoke.IsWindowEnabled(query)) {
                    state=ModernNative.Message(query,pid,0xF2,0,0); // BM_GETSTATE remains available while disabled.
                    if((ProxyUiSmoke.GetWindowLongW(query,-16)&0x08000000)==0) {
                        if(Operation(pinned,pid,parent)["idle"]==1) seen=false;
                        else throw new Exception("Missing WS_DISABLED during actual pending");
                    }
                    if(seen) region=Region(pinned,pid,query);
                } else seen=false;
            }
            var timer=Stopwatch.StartNew(); var done=Operation(pinned,pid,parent);
            while(done["idle"]!=1 || !ProxyUiSmoke.IsWindowEnabled(query)) {
                if(timer.ElapsedMilliseconds>=8000) throw new Exception("Real query completion exceeded eight seconds");
                NoOtherProcess(pinned,pid); Thread.Sleep(2); done=Operation(pinned,pid,parent);
            }
            if(done["sequence"]!=pending["sequence"] || done["generation"]!=before["generation"] || done["rejected"]!=0)
                throw new Exception("Query completion identity/rejection mismatch");
            if(seen) return new PendingProof { Attempts=attempt,Before=before["sequence"],Sequence=done["sequence"],
                Generation=done["generation"],DisabledButtonState=state,PendingSeen=true,Completed=true,
                Frozen=frozen.ToArray(),DisabledRegion=region };
        }
        throw new Exception("Actual pending not observed in 20 bounded native clicks; no pending acceptance claimed");
    }
    public sealed class CropProof {
        public string Path,CornersPath,PhysicalRect;
        public int Width,Height,CornerTile;
        public double MeanLuminance,BrightFraction;
    }
    public static CropProof Crop(string parentPath,StatusBarNative.Rect r,string path,string corners,uint dpi) {
        // Crop a physical parent-client screenshot: unlike a control-only PrintWindow,
        // this includes the parent actually exposed by the clipped nonclient corners.
        using(var parent=new Bitmap(parentPath)) {
            if(r.Left<0 || r.Top<0 || r.Right>parent.Width || r.Bottom>parent.Height || r.Width<4 || r.Height<4)
                throw new Exception("Control/corner evidence clipped by parent client: "+r);
            using(var b=parent.Clone(new Rectangle(r.Left,r.Top,r.Width,r.Height),PixelFormat.Format32bppArgb)) {
                b.Save(path,ImageFormat.Png);
                int tile=Math.Min(Math.Min(b.Width/2,b.Height/2),(int)((16L*dpi+48)/96));
                using(var sheet=new Bitmap(tile*2,tile*2)) {
                    using(var g=Graphics.FromImage(sheet)) for(int y=0;y<2;y++) for(int x=0;x<2;x++) {
                        var src=new Rectangle(x==0?0:b.Width-tile,y==0?0:b.Height-tile,tile,tile);
                        g.DrawImage(b,new Rectangle(x*tile,y*tile,tile,tile),src,GraphicsUnit.Pixel);
                    }
                    sheet.Save(corners,ImageFormat.Png);
                }
                double sum=0; int bright=0,count=0;
                int inset=Math.Max(2,(int)((6L*dpi+48)/96));
                for(int y=inset;y<b.Height-inset;y++) for(int x=inset;x<b.Width-inset;x++) {
                    var c=b.GetPixel(x,y); count++; sum+=.2126*c.R+.7152*c.G+.0722*c.B;
                    if(c.R>=220 && c.G>=220 && c.B>=220) bright++;
                }
                if(count==0) throw new Exception("Empty control interior evidence");
                return new CropProof { Path=path,CornersPath=corners,PhysicalRect=r.ToString(),Width=b.Width,Height=b.Height,
                    CornerTile=tile,MeanLuminance=sum/count,BrightFraction=(double)bright/count };
            }
        }
    }
}
'@)
$joined = $declarations -join "`n"
$usingPattern = '(?m)^using [^;\r\n]+;\r?$'
$usings = @([regex]::Matches($joined,$usingPattern) | ForEach-Object { $_.Value.Trim() } | Select-Object -Unique)
$combined = ($usings -join "`n") + "`n" + [regex]::Replace($joined,$usingPattern,'')
Add-Type -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll,Accessibility.dll -TypeDefinition $combined
if ($ValidateOnly) {
    Write-Output 'PASS: declaration extraction and combined C# compilation only; no build, EXE lookup/launch, mutex, GUI, network, mock, system setting or acceptance test run'
    return
}

$session=[Diagnostics.Process]::GetCurrentProcess().SessionId
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$mutexName="Local\AIRelay.$sid.$session.v1"
$child=$null; $childHandle=[IntPtr]::Zero; $childStart=$null; $ownedPid=0
$window=[IntPtr]::Zero; $temp=$null; $config=$null; $configBytes=$null; $failure=$null
$previousDpi=[IntPtr]::Zero; $stage='preflight'
$checks=[Collections.Generic.List[object]]::new()
$shots=[Collections.Generic.List[object]]::new()
$gdiSamples=[Collections.Generic.List[object]]::new()
$cleanupErrors=[Collections.Generic.List[string]]::new()
$overall=[Diagnostics.Stopwatch]::StartNew()
function Record([string]$Name,[object]$Value) {
    $checks.Add([pscustomobject]@{ Check=$Name; Value=$Value; Seconds=[Math]::Round($overall.Elapsed.TotalSeconds,2) })
    Write-Host ("CHECK {0}: {1}" -f $Name,($Value | ConvertTo-Json -Compress -Depth 7))
}
function Assert-NoOtherProcess {
    $others=@(Get-Process -Name 'ai-relay' -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $ownedPid })
    if ($others.Count -gt 0) { throw "BLOCKED: other ai-relay PID(s) $($others.Id -join ','); no input/IPC/kill sent to them" }
}
function Assert-NoInstance {
    $existing=$null
    try { $existing=[Threading.Mutex]::OpenExisting($mutexName) }
    catch [Threading.WaitHandleCannotBeOpenedException] { }
    if ($null -ne $existing) { $existing.Dispose(); throw 'BLOCKED: same-user/session instance mutex exists; no launch or IPC sent' }
    Assert-NoOtherProcess
}
function Assert-Identity {
    if ($null -eq $child -or $child.HasExited) { throw 'Pinned child missing/exited; refusing input' }
    [StatusBarNative]::CheckProcess($childHandle,[uint32]$ownedPid)
    if ($child.Id -ne $ownedPid -or $child.Handle -ne $childHandle -or $child.StartTime -ne $childStart -or $child.Path -ine $exe) {
        throw 'Child PID/retained handle/start time/executable identity mismatch; refusing input'
    }
}
function Assert-Child {
    Assert-NoOtherProcess
    Assert-Identity
    if ($overall.Elapsed.TotalSeconds -ge 480) { throw "Overall 480-second budget exceeded at $stage" }
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
    while ($timer.Elapsed.TotalSeconds -lt 12) {
        Assert-Child
        if (& $Test) { return }
        Start-Sleep -Milliseconds 30
    }
    throw "Bounded wait expired at $stage / $Name"
}
function Operation {
    $values=@{}
    foreach ($part in (Text 2354).Split(';')) {
        $pair=$part.Split('=')
        if ($pair.Count -eq 2) { $values[$pair[0]]=[uint64]$pair[1] }
    }
    foreach ($key in @('idle','sequence','fence','rejected','generation')) {
        if (-not $values.ContainsKey($key)) { throw 'Missing current history operation state 2354; build latest source first' }
    }
    return $values
}
function Fence {
    $before=(Operation).fence
    [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,2397)
    Wait-State 'owner-thread queue fence' { (Operation).fence -gt $before }
}
function Wait-Idle {
    Fence
    Wait-State 'history idle' { (Operation).idle -eq 1 -and [ProxyUiSmoke]::IsWindowEnabled((Control 2320)) }
    if ((Operation).rejected -ne 0) { throw "Unexpected rejected history command: $(Text 2354)" }
    if ([ProxyUiSmoke]::Dialog([uint32]$ownedPid) -ne [IntPtr]::Zero) { throw 'Unexpected own modal dialog; not automatically answered' }
}
function Segment([int]$Id,[int]$Index) {
    Wait-Idle
    if ([ProxyUiSmoke]::Class((Control $Id)) -ne 'SysTabControl32') { throw "ID $Id must be native segmented Tab" }
    [ModernNative]::ClickTab($childHandle,[uint32]$ownedPid,(Control $Id),$Index)
    Wait-Idle
    if ((Message $Id 0x130B) -ne $Index -or ($Id -ne 2100 -and (Message $Id 0x147) -ne $Index)) { throw "Segment $Id/$Index failed" }
}
function Page([int]$Index,[int]$Expected) {
    Segment 2100 $Index
    Wait-State "page $Index" { Visible $Expected }
}
function Category([int]$Index,[int]$Expected) {
    Page 3 2039
    Message 2039 0x14E ([uint32]$Index) | Out-Null
    [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,[uint32](2039 -bor 0x10000))
    Wait-Idle
    Wait-State "settings category $Index" { Visible $Expected }
}
function Theme([int]$Index) {
    Category 5 2030
    Segment 2030 $Index
    Page 1 2300
    Assert-ConfigUnchanged
}
function Assert-ConfigUnchanged {
    Assert-Child
    if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($config)) -cne $configBytes) { throw 'Theme/input/query/resize wrote config without Save' }
    if ([ProxyUiSmoke]::IsWindowEnabled((Control 2003))) { throw 'Proxy unexpectedly running; networking must remain stopped' }
}
function Edit-State([IntPtr]$Handle) {
    Assert-Child
    [ProxyUiSmoke]::CheckOwner($Handle,[uint32]$ownedPid)
    $style=[ProxyUiSmoke]::GetWindowLongW($Handle,-16)
    $password=($style -band 0x20) -ne 0
    $value=if ($password) { $null } else { [ClarityNative]::Text($Handle,[uint32]$ownedPid) }
    return [pscustomobject]@{
        Handle=$Handle; Password=$password; Text=$value; NativeStyle=($style -band 0x0000FFFF)
        Length=[ModernNative]::Message($Handle,[uint32]$ownedPid,0xE,0,0)
        Selection=[ProxyUiSmoke]::Selection($Handle,[uint32]$ownedPid)
        Mask=[ModernNative]::Message($Handle,[uint32]$ownedPid,0xD2,0,0)
        Limit=[ModernNative]::Message($Handle,[uint32]$ownedPid,0xD5,0,0)
    }
}
function Settings-State {
    Assert-Child
    $states=[Collections.Generic.List[object]]::new()
    foreach ($h in [ModernNative]::DirectChildren($window,[uint32]$ownedPid)) {
        $id=[ClarityNative]::GetDlgCtrlID($h)
        if ([ProxyUiSmoke]::Class($h) -eq 'Edit' -and ($id -eq 0 -or ($id -ge 2040 -and $id -le 2061))) {
            $state=Edit-State $h
            # Private config contains no credentials. Do not bypass Windows password
            # read protection, log content, or claim nonempty-secret retention here.
            if ($state.Password -and ($state.Length -ne 0 -or $state.Mask -eq 0)) { throw 'Expected empty, masked private settings password control' }
            $states.Add($state)
        }
    }
    if ($states.Count -lt 4 -or @($states | Where-Object Password).Count -lt 2) { throw 'Settings Edit/password fixtures missing' }
    return ,$states.ToArray()
}
function Assert-Settings {
    foreach ($old in $settingsBaseline) {
        $now=Edit-State $old.Handle
        foreach ($key in @('Password','Text','NativeStyle','Length','Selection','Mask','Limit')) {
            if ($now.$key -cne $old.$key) { throw "Settings field changed during rounded presentation: $key" }
        }
    }
}
function Assert-Edit {
    $now=Edit-State (Control 2304)
    foreach ($key in @('Text','NativeStyle','Length','Selection','Mask','Limit')) {
        if ($now.$key -cne $editBaseline.$key) { throw "History Edit text/selection/native semantics changed: $key" }
    }
}
function Regions([switch]$Log) {
    foreach ($id in @(2320,2304,2310,2302)) {
        $proof=[RoundedNative]::Region($childHandle,[uint32]$ownedPid,(Control $id))
        if ($Log) { Record "region $id full-window/corners/middle/6-DIP" $proof }
    }
}
function Resize([bool]$Small) {
    Assert-Child
    $requested=[StatusBarNative]::Resize($window,[uint32]$ownedPid,$Small)
    Wait-State 'physical resize completed' {
        $r=[StatusBarNative]::Outer($window,[uint32]$ownedPid)
        $r.Width -eq $requested.Width -and $r.Height -eq $requested.Height
    }
    Fence
    Regions
    Assert-Edit
    return [StatusBarNative]::Outer((Control 2304),[uint32]$ownedPid)
}
function Capture([string]$Name,[IntPtr]$Handle,[switch]$Outer) {
    Assert-Child
    if (-not [ProxyUiSmoke]::IsWindowVisible($Handle)) { throw "Hidden screenshot target: $Name" }
    $shot=[ModernNative]::Capture($Handle,[uint32]$ownedPid,(Join-Path $output "$Name.png"),(-not $Outer),(New-Object StatusBarNative+Rect))
    $actual=if ($Outer) { [StatusBarNative]::Outer($Handle,[uint32]$ownedPid) } else { [StatusBarNative]::Client($Handle,[uint32]$ownedPid) }
    if ($shot.Width -ne $actual.Width -or $shot.Height -ne $actual.Height) { throw 'Screenshot dimensions are not actual physical pixels' }
    $shots.Add([pscustomobject]@{ Name=$Name; DPI=[ClarityNative]::GetDpiForWindow($Handle); PhysicalRect="$actual"; Shot=$shot })
    return $shot
}
function Combo-Open {
    Wait-Idle
    $h=Control 2310
    $info=[ModernNative]::Combo($h,[uint32]$ownedPid)
    [ModernNative]::Mouse($h,[uint32]$ownedPid,([int](($info.Button.Left+$info.Button.Right)/2)),([int](($info.Button.Top+$info.Button.Bottom)/2)))
    Wait-State 'native ComboLBox visible' { (Message 2310 0x157) -eq 1 -and [ProxyUiSmoke]::IsWindowVisible($info.List) }
    if ([ProxyUiSmoke]::Class($info.List) -ne 'ComboLBox') { throw 'Dropdown is not native ComboLBox' }
    return $info.List
}
function Calendar-Handle {
    $h=[IntPtr](Message 2302 0x1008)
    if ($h -ne [IntPtr]::Zero) { [ProxyUiSmoke]::CheckOwner($h,[uint32]$ownedPid) }
    return $h
}

# Preflight before any temporary files, launch, GUI input or IPC. A racing primary
# is also rejected by the app's distinct --config-dir guard; never CLI show/quit.
Assert-NoInstance
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
    throw "Build unavailable: $([IO.Path]::GetFullPath($ExePath)). Wait for build notification; this script never runs Cargo."
}
$exe=(Get-Item -LiteralPath $ExePath).FullName
$outputRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\target\rounded-ui-screenshots'))
$output=Join-Path $outputRoot ([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')+'-'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($output)
try {
    $previousDpi=[StatusBarNative]::EnterDpi()
    if ([DatePickerNative]::HighContrast()) { throw 'System high contrast is active; rounded light/dark acceptance blocked, system settings left unchanged' }
    Record 'high contrast system setting never changed' 'Contrast detach/region removal belongs to isolated rounded/theme tests; NOT run by this script.'
    Assert-NoInstance
    $temp=Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-rounded-'+[Guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($temp)
    $config=Join-Path $temp 'config.toml'
    $toml="schema_version = 1`n[upstream]`nurl = 'http://127.0.0.1:1/v1'`n[listen]`naddress = '127.0.0.1'`nport = 4000`n[proxy]`nauto_start = false`nforward_enabled = false`nconnect_enabled = false`n[ui]`ntheme = 'light'`nremember_window_geometry = false`n[system]`nlogin_auto_start = false`n"
    [IO.File]::WriteAllText($config,$toml,[Text.UTF8Encoding]::new($false))
    $configBytes=[Convert]::ToBase64String([IO.File]::ReadAllBytes($config))
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
        if ($identityTimer.Elapsed.TotalSeconds -ge 5) { throw 'Child executable identity unavailable; no GUI input sent' }
        Start-Sleep -Milliseconds 40
    } while ($true)
    Wait-State 'own main HWND' { $script:window=[ProxyUiSmoke]::Find([uint32]$ownedPid); $window -ne [IntPtr]::Zero }
    Wait-State 'private config loaded with proxy stopped' {
        [ProxyUiSmoke]::IsWindowEnabled((Control 2002)) -and -not [ProxyUiSmoke]::IsWindowEnabled((Control 2003))
    }
    # The top-level HWND/settings precede history creation during startup.
    # Probe existence without treating an expected not-yet-created HWND as foreign.
    Wait-State 'history controls created and operation state published' {
        $h=[ProxyUiSmoke]::GetDlgItem($window,2354)
        $query=[ProxyUiSmoke]::GetDlgItem($window,2320)
        if ($h -eq [IntPtr]::Zero -or $query -eq [IntPtr]::Zero) { return $false }
        [ProxyUiSmoke]::CheckOwner($h,[uint32]$ownedPid)
        [ProxyUiSmoke]::CheckOwner($query,[uint32]$ownedPid)
        return ([ClarityNative]::Text($h,[uint32]$ownedPid)).StartsWith('idle=1;')
    }
    Wait-Idle
    Record 'pinned child, temporary no-autostart config, no mock/network' @{ PID=$ownedPid; Start=$childStart.ToString('o'); Exe=$exe }
    $settingsBaseline=Settings-State
    Assert-ConfigUnchanged
    Page 1 2300
    Segment 2300 0
    Segment 2301 4
    # 2310 belongs to the initially collapsed advanced filters. Open it through
    # the native button, never ShowWindow an otherwise hidden test target.
    if (-not (Visible 2310)) {
        Wait-Idle
        [ModernNative]::Click((Control 2318),[uint32]$ownedPid)
        Wait-Idle
    }
    Wait-State 'advanced native protocol dropdown visible' { Visible 2310 }
    foreach ($pair in @(@(2320,'Button'),@(2304,'Edit'),@(2310,'ComboBox'),@(2302,'SysDateTimePick32'))) {
        if ([ProxyUiSmoke]::Class((Control $pair[0])) -ne $pair[1]) { throw "Wrong native class for ID $($pair[0])" }
    }
    if (([ProxyUiSmoke]::GetWindowLongW((Control 2304),-16) -band (4 -bor 0x00300000)) -ne 0) { throw '2304 is not a native single-line, non-scrolling Edit' }
    if (([ProxyUiSmoke]::GetWindowLongW((Control 2310),-16) -band 3) -ne 3) { throw '2310 must retain CBS_DROPDOWNLIST' }
    $dateStyle=[ProxyUiSmoke]::GetWindowLongW((Control 2302),-16) -band 0xFFFF
    if (($dateStyle -band 2) -eq 0) { throw '2302 must retain DTS_SHOWNONE' }
    $stage='native WM_SETTEXT, keyboard, selection and caret'
    [ProxyUiSmoke]::SetText((Control 2304),[uint32]$ownedPid,'127.0.0.1')
    if ((Text 2304) -cne '127.0.0.1') { throw 'Edit WM_SETTEXT failed' }
    [ModernNative]::Click((Control 2304),[uint32]$ownedPid)
    Wait-State 'native Edit focus and caret' { [RoundedNative]::Caret($childHandle,[uint32]$ownedPid,(Control 2304)) }
    [DatePickerNative]::Key($childHandle,[uint32]$ownedPid,(Control 2304),0x23) # End
    Fence
    [DatePickerNative]::Key($childHandle,[uint32]$ownedPid,(Control 2304),0x25) # Left
    Fence
    [DatePickerNative]::Key($childHandle,[uint32]$ownedPid,(Control 2304),0x2E) # Delete via native Edit key handling.
    Wait-State 'native Delete removes final digit' { (Text 2304) -ceq '127.0.0.' }
    [ProxyUiSmoke]::Post((Control 2304),[uint32]$ownedPid,0x102,50) # HWND-local WM_CHAR '2', not global input.
    Wait-State 'native character insertion' { (Text 2304) -ceq '127.0.0.2' }
    Message 2304 0xB1 2 7 | Out-Null # EM_SETSEL is scalar; EM_GETSEL uses its packed return, no pointers.
    $editBaseline=Edit-State (Control 2304)
    if ($editBaseline.Selection -ne (2 -bor (7 -shl 16))) { throw 'Native selection range failed' }
    Record 'Edit WM_SETTEXT + End/Left/Delete/WM_CHAR, caret and selection 2..7' @{ Text=$editBaseline.Text; Selection=$editBaseline.Selection }
    # Two actual resize sizes are required, not a clamped no-op presented as success.
    $large=Resize $false
    $small=Resize $true
    if ($large.Width -eq $small.Width -and $large.Height -eq $small.Height) { throw 'Monitor clamped both sizes; region resize acceptance not demonstrated' }
    Record 'resized Edit full-window region in distinct physical sizes' @{ Large="$large"; Small="$small" }
    if (-not [DatePickerNative]::SetISO($childHandle,[uint32]$ownedPid,(Control 2302),'2024-02-29')) { throw 'Private date fixture rejected' }
    $dateText=Text 2302

    $stage='native combobox dropdown and keyboard selection'
    $list=Combo-Open
    [DatePickerNative]::Key($childHandle,[uint32]$ownedPid,(Control 2310),0x24) # Home
    Fence
    [DatePickerNative]::Key($childHandle,[uint32]$ownedPid,(Control 2310),0x28) # Down
    Wait-State 'native dropdown selection index 1' { (Message 2310 0x147) -eq 1 }
    [DatePickerNative]::Key($childHandle,[uint32]$ownedPid,(Control 2310),0x0D)
    Wait-State 'native dropdown committed and closed' { (Message 2310 0x157) -eq 0 -and -not [ProxyUiSmoke]::IsWindowVisible($list) }
    if ((Message 2310 0x147) -ne 1) { throw 'Native combo selection lost after Enter' }
    Record '2310 actual ComboLBox, Home/Down/Enter native selection' 1

    foreach ($theme in @(@(1,'light'),@(2,'dark'))) {
        $stage="$($theme[1]) physical evidence and native popup"
        $name=$theme[1]
        Theme $theme[0]
        if ((Message 2310 0x147) -ne 1 -or (Text 2302) -cne $dateText) { throw 'Theme reset combo/date value' }
        Assert-Edit
        Assert-Settings
        Regions -Log
        foreach ($id in @(2320,2321,2322)) {
            if (([ProxyUiSmoke]::GetWindowLongW((Control $id),-16) -band 15) -ne 11) { throw "Push button $id must be owner-drawn in both light and dark modes" }
        }
        $parent=Capture "$name-history" $window
        Capture "$name-history-window" $window -Outer | Out-Null
        foreach ($id in @(2320,2304,2310,2302,2321)) {
            $r=[StatusBarNative]::InParent((Control $id),$window,[uint32]$ownedPid)
            $crop=[RoundedNative]::Crop($parent.Path,$r,(Join-Path $output "$name-$id.png"),(Join-Path $output "$name-$id-corners.png"),[ClarityNative]::GetDpiForWindow((Control $id)))
            $shots.Add([pscustomobject]@{ Name="$name-$id-full-and-four-physical-corners"; Shot=$crop })
            if ($name -eq 'dark' -and $id -in @(2304,2310,2302) -and ($crop.MeanLuminance -gt 135 -or $crop.BrightFraction -gt .12)) { throw "Dark closed native control $id has a large light surface" }
        }
        [ModernNative]::Click((Control 2304),[uint32]$ownedPid)
        Message 2304 0xB1 2 7 | Out-Null
        Wait-State 'themed Edit native caret retained' { [RoundedNative]::Caret($childHandle,[uint32]$ownedPid,(Control 2304)) }
        $focused=Capture "$name-history-edit-focused" $window
        $focusRect=[StatusBarNative]::InParent((Control 2304),$window,[uint32]$ownedPid)
        $focusCrop=[RoundedNative]::Crop($focused.Path,$focusRect,(Join-Path $output "$name-edit-focused.png"),(Join-Path $output "$name-edit-focused-corners.png"),[ClarityNative]::GetDpiForWindow((Control 2304)))
        $shots.Add([pscustomobject]@{ Name="$name-edit-focused-native-caret"; Shot=$focusCrop })
        Assert-Edit
        Record "$name native focused Edit caret and selection" $true
        $list=Combo-Open
        Capture "$name-combo-dropdown-native" $list -Outer | Out-Null
        [DatePickerNative]::Key($childHandle,[uint32]$ownedPid,(Control 2310),0x1B)
        Wait-State 'combo Escape closes popup' { (Message 2310 0x157) -eq 0 }
        if ((Message 2310 0x147) -ne 1) { throw 'Combo popup roundtrip changed selection' }
        [DatePickerNative]::MousePart($childHandle,[uint32]$ownedPid,(Control 2302),$true)
        Wait-State '2302 actual month-calendar popup' { (Calendar-Handle) -ne [IntPtr]::Zero -and [ProxyUiSmoke]::IsWindowVisible((Calendar-Handle)) }
        $calendar=Calendar-Handle
        if ([ProxyUiSmoke]::Class($calendar) -ne 'SysMonthCal32') { throw 'Date dropdown is not a native month calendar' }
        $minimum=[DatePickerNative]::CalendarMinimum($childHandle,[uint32]$ownedPid,$calendar)
        $client=[StatusBarNative]::Client($calendar,[uint32]$ownedPid)
        if ($client.Width -lt $minimum.Width -or $client.Height -lt $minimum.Height) { throw 'Native calendar popup clipped below MCM_GETMINREQRECT' }
        Capture "$name-date-native-popup" $calendar -Outer | Out-Null
        [DatePickerNative]::Key($childHandle,[uint32]$ownedPid,$calendar,0x1B)
        Fence
        if ((Calendar-Handle) -ne [IntPtr]::Zero) { Message 2302 0x100D | Out-Null }
        Wait-State 'native date popup closed' { (Calendar-Handle) -eq [IntPtr]::Zero }
        if ((Text 2302) -cne $dateText -or ([ProxyUiSmoke]::GetWindowLongW((Control 2302),-16) -band 0xFFFF) -ne $dateStyle) { throw 'Date native style/value changed' }
        Record "$name native calendar popup preserves styles/value" @{ Client="$client"; Minimum="$minimum"; Note='Native visual styles retained; light popup in dark mode is allowed.' }
        Page 0 2109
        Capture "$name-overview" $window | Out-Null
        Capture "$name-overview-window" $window -Outer | Out-Null
        Page 1 2300
        Assert-ConfigUnchanged
    }

    $stage='real native query pending -> completion, no records'
    Wait-Idle
    $pending=[RoundedNative]::Query($childHandle,[uint32]$ownedPid,$window)
    Wait-Idle
    if (-not $pending.PendingSeen -or -not $pending.Completed -or -not (Text 2325).Contains('匹配 0 请求') -or (Message 2337 0x1004) -ne 0) {
        throw "Real empty query/pending/completion failed: $(Text 2325)"
    }
    Assert-Edit
    Assert-Settings
    Record 'genuine native button query, observed disabled pending and matching completion' $pending
    # Stable disabled previous/next buttons provide paint/state evidence after completion.
    $sequence=(Operation).sequence
    foreach ($id in @(2321,2322)) {
        if ([ProxyUiSmoke]::IsWindowEnabled((Control $id)) -or ([ProxyUiSmoke]::GetWindowLongW((Control $id),-16) -band 0x08000000) -eq 0) { throw "Empty-history navigation button $id must be disabled" }
        $state=Message $id 0xF2
        # BM_CLICK is programmatic activation, not physical disabled-input routing;
        # native BUTTON implementations can still emit BN_CLICKED when sent it.
        # Disabled acceptance here is intentionally read-only: do not inject actions.
        Fence
        if ((Operation).sequence -ne $sequence) { throw 'Read-only disabled-state inspection submitted a query' }
        Record "disabled native button $id WS_DISABLED/BM_GETSTATE + retained region" @{ State=$state; Region=[RoundedNative]::Region($childHandle,[uint32]$ownedPid,(Control $id)) }
    }

    $stage='GDI warmup and 100 actual theme+resize cycles'
    for ($i=0;$i -lt 4;$i++) {
        Theme (1+($i%2))
        Resize (($i%2) -eq 1) | Out-Null
    }
    Assert-Settings
    $gdiBefore=[StatusBarNative]::Gdi($childHandle,[uint32]$ownedPid)
    for ($i=0;$i -lt 100;$i++) {
        Theme (1+($i%2))
        Resize (($i%2) -eq 1) | Out-Null
        $count=[StatusBarNative]::Gdi($childHandle,[uint32]$ownedPid)
        $gdiSamples.Add([pscustomobject]@{ Cycle=($i+1); Theme=(1+($i%2)); Small=(($i%2) -eq 1); GDI=$count })
        if ($count -gt $gdiBefore+8) { throw "Unbounded GDI growth at cycle $($i+1): $count vs warmed baseline $gdiBefore" }
        if (($i+1)%10 -eq 0) { Assert-Settings; Write-Host "GDI cycle $($i+1)/100: $count (baseline $gdiBefore)" }
    }
    Wait-Idle
    $gdiAfter=[StatusBarNative]::Gdi($childHandle,[uint32]$ownedPid)
    if ($gdiAfter -gt $gdiBefore) { throw "GDI increased after 100 cycles at identical theme/size: $gdiBefore -> $gdiAfter" }
    Record '100 theme+resize cycles bounded; final GDI has no increase' @{ Before=$gdiBefore; After=$gdiAfter; Peak=($gdiSamples | Measure-Object GDI -Maximum).Maximum; TransientLimit=8 }
    Assert-Edit
    Assert-Settings
    if ((Message 2310 0x147) -ne 1 -or (Text 2302) -cne $dateText) { throw 'Stress loop lost native selection/date' }
    Assert-ConfigUnchanged
    $stage='clean exact-child exit'
    Assert-Child
    [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,1002)
    if (-not $child.WaitForExit(38000) -or $child.ExitCode -ne 0) { throw 'Exact child clean exit failed/timed out; never killed' }
    if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($config)) -cne $configBytes) { throw 'Clean exit wrote unsaved config' }
    Record 'clean exact-child exit and byte-identical config' $ownedPid
} catch {
    $failure=$_
    Write-Warning ("FAIL stage={0}; {1}; {2}" -f $stage,$_.Exception.Message,$_.InvocationInfo.PositionMessage)
} finally {
    $exited=$null -eq $child
    if ($null -ne $child) {
        try {
            if (-not $child.HasExited) {
                # A newly appeared instance stops tests, but cleanup may address ONLY
                # the exact retained child. Never CLI quit/stop or process-name kill.
                Assert-Identity
                if ($window -eq [IntPtr]::Zero) { $window=[ProxyUiSmoke]::Find([uint32]$ownedPid) }
                if ($window -ne [IntPtr]::Zero) { [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,1002) }
                if (-not $child.WaitForExit(38000)) { throw 'Own cleanup timed out; child left alive and temp retained; no kill' }
            }
            $exited=$child.HasExited
        } catch { $cleanupErrors.Add($_.Exception.Message) }
        finally { $child.Dispose(); $child=$null }
    }
    if ($previousDpi -ne [IntPtr]::Zero) { [StatusBarNative]::RestoreDpi($previousDpi) }
    if ($null -ne $temp -and $exited) {
        for ($attempt=0;$attempt -lt 10;$attempt++) {
            try { [IO.Directory]::Delete($temp,$true); break }
            catch {
                if ($attempt -eq 9) { $cleanupErrors.Add("Temp cleanup failed; retained $temp : $($_.Exception.Message)") }
                else { Start-Sleep -Milliseconds 150 }
            }
        }
    } elseif ($null -ne $temp) { $cleanupErrors.Add("Child exit unconfirmed; temp retained at $temp") }
    $detail=$null
    if ($null -ne $failure) { $detail=[pscustomobject]@{ Stage=$stage; Message=$failure.Exception.Message; Position=$failure.InvocationInfo.PositionMessage; Stack=$failure.ScriptStackTrace } }
    $report=[pscustomobject]@{
        Passed=($null -eq $failure -and $cleanupErrors.Count -eq 0); Exe=$exe; PID=$ownedPid; Failure=$detail
        CleanupErrors=@($cleanupErrors.ToArray()); Checks=@($checks.ToArray()); Screenshots=@($shots.ToArray()); GDI=@($gdiSamples.ToArray())
        Seconds=[Math]::Round($overall.Elapsed.TotalSeconds,2)
        Limits='Only current physical monitor DPI; no multi-monitor/DPI matrix, physical keyboard/SendInput, screen reader or system high-contrast switch. High-contrast detach/region-removal tests are NOT run here. Targeted native key/character messages are not physical keyboard acceptance. Password settings are initially empty: mask/length/selection/style retention only, not proof for nonempty secrets. Corner crops are visual evidence, not automated pixel-perfect surface/caret rendering proof; local GDI region geometry is checked independently. Native focused date/month-calendar visual styles may remain light in dark mode. Empty private history only; no record-boundary membership/performance/load acceptance. No Cargo, mock, network requests, config Save, Startup/firewall changes, global input, other-instance IPC or kill.'
    }
    [IO.File]::WriteAllText((Join-Path $output 'results.json'),($report | ConvertTo-Json -Depth 12),[Text.UTF8Encoding]::new($true))
    Write-Output "Rounded UI artifacts: $output"
}
if ($null -ne $failure) { throw $failure }
if ($cleanupErrors.Count -gt 0) { throw ($cleanupErrors -join '; ') }
Write-Output 'PASS: 6-DIP rounded native regions/full-window resize, Edit text/selection/caret, unchanged empty masked settings, native combo/calendar popup, observed real pending/completion with zero records, disabled buttons, light/dark physical corner/overview/history evidence, 100 bounded GDI theme+resize cycles and byte-identical config.'
