# Windows PowerShell 5.1 x64; isolated, stopped GUI fixture only.
# Usage: powershell.exe -NoProfile -File tools\smoke-ui-regressions.ps1 -ValidateOnly
# Then run without -ValidateOnly on an interactive desktop. No build/save/network.
# Only declarations from the tracked title-menu smoke are reused; its body never runs.
# Screenshots/reports go under target/review-ui; temporary data is retained for audit.
# Ownership: retain the Process object and its kernel Handle/StartTime/EXE; verify
# them and all ai-relay* processes before interaction. HWNDs are rediscovered below
# that pinned main window, recursively through viewport 2400. Choice WM_COMMAND
# goes to GetParent(choice), not an assumed main HWND; no WM_NOTIFY pointer crosses
# process boundaries. Only the owned main receives exit command 1002; never kill.
# Physical captures are client-only and refuse foreign z-order overlap before and
# after CopyFromScreen. Residual tests never substitute PrintWindow for acceptance.
# Complete means this bounded 96-DPI matrix, not full physical keyboard/IME,
# high-contrast, 150/200% DPI, multi-monitor or screen-reader product acceptance.
[CmdletBinding()]
param(
    [string]$ExePath,
    [switch]$ValidateOnly,
    [ValidateRange(30,120)][int]$BudgetSeconds = 120
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([IntPtr]::Size -ne 8) { throw 'Use a fresh x64 powershell.exe -NoProfile' }
if ('TitleMenuSmoke.Native' -as [type]) { throw 'Helpers already loaded; use a fresh PowerShell process' }
$project = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\target\debug\ai-relay.exe'))
}
$source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'smoke-title-menu-ui.ps1'))
$declarations = [regex]::Matches($source, '(?s)\$nativeSource = @''\r?\n(.*?)\r?\n''@')
if ($declarations.Count -ne 1) { throw 'Expected exactly one title-menu C# declaration; no script body executed' }
$extra = @'
namespace UiRegression {
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Runtime.InteropServices;
using Accessibility;
using N = TitleMenuSmoke.Native;
public static class Native {
    [StructLayout(LayoutKind.Sequential)] struct R { public int Left,Top,Right,Bottom; }
    [StructLayout(LayoutKind.Sequential)] struct P { public int X,Y; }
    delegate bool EnumProc(IntPtr h,IntPtr unused);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int w,int height,uint flags);
    [DllImport("user32.dll")] static extern bool RedrawWindow(IntPtr h,IntPtr rect,IntPtr region,uint flags);
    [DllImport("user32.dll")] static extern int GetWindowRgn(IntPtr h,IntPtr r);
    [DllImport("user32.dll")] static extern int GetWindowLongW(IntPtr h,int index);
    [DllImport("gdi32.dll")] static extern IntPtr CreateRectRgn(int l,int t,int r,int b);
    [DllImport("gdi32.dll")] static extern IntPtr CreateRoundRectRgn(int l,int t,int r,int b,int w,int h);
    [DllImport("gdi32.dll")] static extern bool EqualRgn(IntPtr a,IntPtr b);
    [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr r);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(P p);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc p,IntPtr unused);
    [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h,out R r);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h,uint attr,out uint value,uint size);
    [DllImport("oleacc.dll")] static extern int AccessibleObjectFromWindow(IntPtr h,uint id,ref Guid iid,[MarshalAs(UnmanagedType.Interface)] out IAccessible accessible);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
    [StructLayout(LayoutKind.Sequential)] struct GUI { public uint Size,Flags; public IntPtr Active,Focus,Capture,Menu,Move,Caret; public R Rect; }
    [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint thread,ref GUI info);
    public sealed class EditState {
        public long Handle,Style,ExStyle,Selection,Margins,Mask,Limit,Length;
        public string Text,NativeFrame; public bool Password;
    }
    public sealed class Difference { public long Pixels; public string Box; public int Compared; }
    public static void Resize(IntPtr h,uint pid,int w,int height) {
        N.Check(h,pid);
        if(!SetWindowPos(h,IntPtr.Zero,20,20,w,height,0x10)) throw new Exception("Owned SetWindowPos failed");
        var r=N.Outer(h,pid); if(r.Width!=w || r.Height!=height) throw new Exception("Actual outer dimensions != requested dimensions");
    }
    public static void Repaint(IntPtr h,uint pid) {
        N.Check(h,pid); if(!RedrawWindow(h,IntPtr.Zero,IntPtr.Zero,0x485)) throw new Exception("RedrawWindow 0x485 failed");
    }
    public static void Tab(IntPtr h,uint pid,int index) {
        N.Check(h,pid); int p=(18<<16)|(index*112+56);
        N.Message(h,pid,0x201,1,p); N.Message(h,pid,0x202,0,p);
    }
    public static void NoEditRegion(IntPtr h,uint pid) {
        N.Check(h,pid); if(N.Class(h)!="Edit") throw new Exception("Not a native Edit");
        var r=CreateRectRgn(0,0,0,0); if(r==IntPtr.Zero) throw new Exception("Local region allocation failed");
        try { if(GetWindowRgn(h,r)!=0) throw new Exception("REGRESSION: Edit has an application-owned clipping region"); }
        finally { DeleteObject(r); }
    }
    public static EditState Edit(IntPtr h,uint pid) {
        NoEditRegion(h,pid);
        long style=unchecked((uint)GetWindowLongW(h,-16))&0xFFFFFF;
        long ex=unchecked((uint)GetWindowLongW(h,-20));
        // The themed Edit consumes WS_BORDER and can paint the native border
                // inside its client. Neither style nor non-client inset proves pixels.
                // Record both; save native Edit crops for actual visual review below.
                var outer=N.Outer(h,pid); var client=N.Client(h,pid);
        bool password=(style&0x20)!=0;
        return new EditState { Handle=h.ToInt64(),Style=style,ExStyle=ex,Password=password,NativeFrame="outer="+outer+"; client="+client,
            Text=password?null:N.Text(h,pid),Selection=N.Message(h,pid,0xB0,0,0),
            Margins=N.Message(h,pid,0xD4,0,0),Mask=N.Message(h,pid,0xD2,0,0),
            Limit=N.Message(h,pid,0xD5,0,0),Length=N.Message(h,pid,14,0,0) };
    }
    public static string Rounded(IntPtr h,uint pid) {
        N.Check(h,pid); var box=N.Outer(h,pid); int d=Math.Min(Math.Min(box.Width,box.Height),(int)((12*N.GetDpiForWindow(h)+48)/96));
        var actual=CreateRectRgn(0,0,0,0); var expected=CreateRoundRectRgn(0,0,box.Width+1,box.Height+1,d,d);
        if(actual==IntPtr.Zero || expected==IntPtr.Zero) throw new Exception("Region allocation failed");
        try { if(GetWindowRgn(h,actual)!=3 || !EqualRgn(actual,expected)) throw new Exception("REGRESSION: rounded button/combo/date region differs from native 6-DIP raster"); }
        finally { DeleteObject(actual); DeleteObject(expected); }
        return N.Class(h)+"; "+box+"; exact 6-DIP rounded region";
    }
    static Rectangle ClientScreen(IntPtr h,uint pid) {
        var r=N.Client(h,pid); var p=N.Origin(h,pid);
        if(r.Width<1 || r.Height<1 || r.Width>4096 || r.Height>4096) throw new Exception("Capture bounds invalid");
        return new Rectangle(p.X,p.Y,r.Width,r.Height);
    }
    static void Unoccluded(IntPtr main,uint pid,Rectangle rect) {
        N.Check(main,pid); bool reached=false; string obstruction=null;
        // Reject any visible foreign top-level rectangle above our window, not just
        // sampled points. Cloaked/minimized windows are not physical obstructions.
        EnumWindows(delegate(IntPtr h,IntPtr unused) {
            if(h==main) { reached=true; return false; }
            if(N.Owner(h)!=pid && N.IsWindowVisible(h) && !IsIconic(h)) {
                uint cloaked; R r;
                if(DwmGetWindowAttribute(h,14,out cloaked,4)!=0) cloaked=0;
                if(cloaked==0 && GetWindowRect(h,out r) && Rectangle.Intersect(rect,new Rectangle(r.Left,r.Top,Math.Max(0,r.Right-r.Left),Math.Max(0,r.Bottom-r.Top))).Width>0 && Rectangle.Intersect(rect,new Rectangle(r.Left,r.Top,Math.Max(0,r.Right-r.Left),Math.Max(0,r.Bottom-r.Top))).Height>0) obstruction="foreign top-level overlap: class="+N.Class(h)+"; pid="+N.Owner(h);
            }
            return true;
        },IntPtr.Zero);
        if(!reached || obstruction!=null) throw new Exception("SKIP physical capture: "+(obstruction??"owned window missing from z-order"));
        for(int y=rect.Top;y<rect.Bottom;y+=12) for(int x=rect.Left;x<rect.Right;x+=12)
            if(N.Owner(WindowFromPoint(new P { X=x,Y=y }))!=pid) throw new Exception("SKIP physical capture: occluded/offscreen owned client");
    }
    public static string Screen(IntPtr main,uint pid,string path) {
        var rect=ClientScreen(main,pid); Unoccluded(main,pid,rect);
        using(var b=new Bitmap(rect.Width,rect.Height,PixelFormat.Format32bppArgb)) {
            using(var g=Graphics.FromImage(b)) g.CopyFromScreen(rect.Left,rect.Top,0,0,b.Size);
            Unoccluded(main,pid,rect);
            if(ClientScreen(main,pid)!=rect) throw new Exception("SKIP physical capture: window moved during capture");
            b.Save(path,ImageFormat.Png);
        }
        return rect.ToString();
    }
    public static Rectangle Relative(IntPtr main,IntPtr h,uint pid) {
        N.Check(h,pid); var r=N.Outer(h,pid); var p=N.Origin(main,pid);
        return new Rectangle(r.Left-p.X,r.Top-p.Y,r.Width,r.Height);
    }
    public static void Crop(string input,string output,Rectangle rect) {
        using(var b=new Bitmap(input)) {
            if(!new Rectangle(Point.Empty,b.Size).Contains(rect)) throw new Exception("Crop outside owned capture");
            using(var c=b.Clone(rect,PixelFormat.Format32bppArgb)) c.Save(output,ImageFormat.Png);
        }
    }
    public static string NativeBorder(string image,Rectangle rect) {
            using(var b=new Bitmap(image)) {
                if(!new Rectangle(Point.Empty,b.Size).Contains(rect) || rect.Width<4 || rect.Height<4) throw new Exception("Border crop out of bounds");
                var color=b.GetPixel(rect.Left,rect.Top); int n=0;
                for(int x=rect.Left;x<rect.Right;x++) {
                    if(b.GetPixel(x,rect.Top)!=color || b.GetPixel(x,rect.Bottom-1)!=color) throw new Exception("REGRESSION: native Edit horizontal border not continuous"); n+=2;
                }
                for(int y=rect.Top;y<rect.Bottom;y++) {
                    if(b.GetPixel(rect.Left,y)!=color || b.GetPixel(rect.Right-1,y)!=color) throw new Exception("REGRESSION: native Edit vertical border not continuous"); n+=2;
                }
                if(b.GetPixel(rect.Right-3,rect.Top+3)==color) throw new Exception("Native border indistinguishable from field interior");
                return String.Format("Continuous rectangular native border; {0} perimeter samples; RGB={1},{2},{3}; distinct interior",n,color.R,color.G,color.B);
            }
        }
        public static Difference Diff(string before,string after,string output,Rectangle area,Rectangle[] excluded) {
        using(var a=new Bitmap(before)) using(var b=new Bitmap(after)) using(var diff=new Bitmap(a.Width,a.Height)) {
            if(a.Size!=b.Size) throw new Exception("Physical screenshot geometry changed");
            long n=0; int count=0,left=a.Width,top=a.Height,right=0,bottom=0;
            for(int y=area.Top;y<area.Bottom;y++) for(int x=area.Left;x<area.Right;x++) {
                bool skip=false; foreach(var e in excluded) if(e.Contains(x,y)) { skip=true; break; }
                if(skip) continue; count++;
                if(a.GetPixel(x,y)!=b.GetPixel(x,y)) { n++; left=Math.Min(left,x); top=Math.Min(top,y); right=Math.Max(right,x); bottom=Math.Max(bottom,y); diff.SetPixel(x,y,Color.Magenta); }
            }
            diff.Save(output,ImageFormat.Png);
            return new Difference { Pixels=n,Compared=count,Box=n==0?"none":String.Format("{0},{1}..{2},{3}",left,top,right,bottom) };
        }
    }
    public static string[] IconProof(IntPtr main,IntPtr tabs,uint pid,string image,string maskDirectory,string prefix,string theme) {
        N.Check(main,pid); N.Check(tabs,pid);
        Guid iid=new Guid("618736E0-3C3D-11CF-810C-00AA00389B71"); IAccessible acc;
        if(AccessibleObjectFromWindow(tabs,0xFFFFFFFC,ref iid,out acc)!=0 || acc==null) throw new Exception("Native Tab accessibility unavailable");
        string[] assets={"overview","analytics","logs","settings"}; string[] labels={"概览","统计分析","实时日志","设置"};
        var proofs=new List<string>(); var tabBox=N.Outer(tabs,pid); var origin=N.Origin(main,pid);
        try { using(var screen=new Bitmap(image)) {
            for(int i=0;i<4;i++) {
                int x,y,w,h; acc.accLocation(out x,out y,out w,out h,i+1);
                string name=acc.get_accName(i+1); var item=new Rectangle(x-origin.X,y-origin.Y,w,h);
                if(name!=labels[i] || !new Rectangle(tabBox.Left-origin.X,tabBox.Top-origin.Y,tabBox.Width,tabBox.Height).Contains(item)) throw new Exception("Native tab bounds/name mismatch or clipped item");
                byte[] mask=File.ReadAllBytes(Path.Combine(maskDirectory,assets[i]+".mask"));
                if(mask.Length<334 || mask[0]!=78 || mask[1]!=65 || mask[2]!=86 || mask[3]!=49 || BitConverter.ToUInt16(mask,4)!=18 || BitConverter.ToUInt32(mask,6)!=324) throw new Exception("Unexpected SVG-derived NAV1 mask");
                bool active=N.Message(tabs,pid,0x130B,0,0)==i;
                Color fg=theme=="dark"?(active?Color.FromArgb(155,186,255):Color.FromArgb(161,175,197)):(active?Color.FromArgb(49,92,206):Color.FromArgb(90,106,128));
                Color bg=theme=="dark"?(active?Color.FromArgb(41,57,81):Color.FromArgb(21,25,34)):(active?Color.FromArgb(232,239,255):Color.FromArgb(245,247,251));
                int best=Int32.MaxValue,bx=0,by=0;
                int cy=item.Top+(item.Height-18)/2;
                for(int sy=cy-1;sy<=cy+1;sy++) for(int sx=item.Left+4;sx<=item.Right-22;sx++) {
                    int bad=0;
                    for(int row=0;row<18;row++) for(int col=0;col<18;col++) {
                        int alpha=mask[10+row*18+col]; var p=screen.GetPixel(sx+col,sy+row);
                        int er=(fg.R*alpha+bg.R*(255-alpha)+127)/255,eg=(fg.G*alpha+bg.G*(255-alpha)+127)/255,eb=(fg.B*alpha+bg.B*(255-alpha)+127)/255;
                        if(Math.Abs(p.R-er)>2 || Math.Abs(p.G-eg)>2 || Math.Abs(p.B-eb)>2) bad++;
                    }
                    if(bad<best) { best=bad; bx=sx; by=sy; }
                }
                Crop(image,prefix+"-"+assets[i]+".png",item);
                if(best!=0) throw new Exception("REGRESSION: SVG-derived "+assets[i]+" icon absent/clipped/incorrect; mismatching pixels="+best);
                proofs.Add(assets[i]+" item="+item+"; icon="+new Rectangle(bx,by,18,18)+"; all 324 mask pixels match within 2 levels");
            }
        } } finally { Marshal.ReleaseComObject(acc); }
        return proofs.ToArray();
    }
    public static IntPtr Focus(IntPtr main,uint pid) {
        N.Check(main,pid); uint actual; var info=new GUI { Size=(uint)Marshal.SizeOf(typeof(GUI)) };
        if(!GetGUIThreadInfo(GetWindowThreadProcessId(main,out actual),ref info)) throw new Exception("GUI focus query failed");
        if(info.Focus!=IntPtr.Zero) N.Check(info.Focus,pid); return info.Focus;
    }
}
}
'@
Add-Type -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll,Accessibility.dll -TypeDefinition ($declarations[0].Groups[1].Value + "`n" + $extra)
$layout = [TitleMenuSmoke.Native]::Validate()
$tokens=$null; $parseErrors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($PSCommandPath,[ref]$tokens,[ref]$parseErrors)
if ($parseErrors.Count) { throw 'PowerShell parser errors' }
foreach ($typeName in @('TitleMenuSmoke.Native','UiRegression.Native')) {
    $calls=$ast.FindAll({ param($n) $n -is [Management.Automation.Language.InvokeMemberExpressionAst] -and $n.Static -and $n.Expression.Extent.Text -eq "[$typeName]" },$true)
    foreach ($call in $calls) {
        if (@(([type]$typeName).GetMethods() | Where-Object { $_.IsStatic -and $_.Name -eq $call.Member.Value }).Count -eq 0) { throw "Missing public helper $typeName.$($call.Member.Value)" }
    }
}
if ($ValidateOnly) {
    Write-Output "PASS: parser, public static call sites, combined C# compilation; $layout"
    Write-Output 'VALIDATION ONLY: no EXE lookup/build, files, mutex, launch, GUI, network, Explorer or system changes'
    return
}
if (-not [IO.Path]::IsPathRooted($ExePath)) { throw 'ExePath must be absolute; current directory is not a path base' }
$exe=[IO.Path]::GetFullPath($ExePath)
if ([IO.Path]::GetFileNameWithoutExtension($exe) -notlike 'ai-relay*') { throw 'Expected ai-relay* executable' }
$session=[Diagnostics.Process]::GetCurrentProcess().SessionId
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$mutexName="Local\AIRelay.$sid.$session.v1"
$ownedPid=0; $child=$null; $childHandle=[IntPtr]::Zero; $childStart=$null
$window=[IntPtr]::Zero; $oldDpi=[IntPtr]::Zero; $clock=$null; $normalExit=$false; $failure=$null; $stage='preflight'
$events=[Collections.Generic.List[object]]::new()
$output=Join-Path $project ('target\review-ui\regressions-'+[DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')+'-'+[Guid]::NewGuid().ToString('N').Substring(0,6))
function Record($Name,$Value) { $events.Add([pscustomobject]@{Name=$Name;Value=$Value}); Write-Host ($Name+': '+($Value|ConvertTo-Json -Depth 10 -Compress)) }
function Assert-True([bool]$Condition,[string]$Message) { if(-not $Condition) { throw ('REGRESSION: '+$Message) } }
function No-Others {
    $others=@(Get-Process -Name 'ai-relay*' -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $ownedPid })
    if($others.Count) { throw ('Foreign ai-relay* exists; no launch/input/IPC/kill: '+($others.Id -join ',')) }
}
function No-Instance {
    No-Others; $m=$null
    try { $m=[Threading.Mutex]::OpenExisting($mutexName) } catch [Threading.WaitHandleCannotBeOpenedException] {}
    if($null -ne $m) { $m.Dispose(); throw 'Existing user/session mutex; launch refused' }
}
function Identity {
    No-Others
    if($null -eq $child -or $child.HasExited) { throw 'Owned child is not live' }
    [TitleMenuSmoke.Native]::CheckProcess($childHandle,[uint32]$ownedPid)
    if($child.Handle -ne $childHandle -or $child.StartTime -ne $childStart -or $child.Path -ine $exe) { throw 'Pinned PID/start/exe/handle mismatch' }
    if($window -ne [IntPtr]::Zero) { [TitleMenuSmoke.Native]::Check($window,[uint32]$ownedPid) }
}
function Guard { Identity; if($clock.Elapsed.TotalSeconds -ge ($BudgetSeconds-8)) { throw 'GUI budget reached; reserving 8s for owned command-1002 exit' } }
function Control([int]$Id) {
    Guard; $h=[TitleMenuSmoke.Native]::FindControl($window,[uint32]$ownedPid,$Id)
    if($h -eq [IntPtr]::Zero) { throw "Missing control $Id" }; return $h
}
function Msg([IntPtr]$Handle,[uint32]$Code,[uint64]$W=0,[long]$L=0) { Guard; [TitleMenuSmoke.Native]::Message($Handle,[uint32]$ownedPid,$Code,$W,$L) }
function Text([int]$Id) { Guard; [TitleMenuSmoke.Native]::Text((Control $Id),[uint32]$ownedPid) }
function Page([int]$Index) {
    Guard; [UiRegression.Native]::Tab((Control 2100),[uint32]$ownedPid,$Index); Start-Sleep -Milliseconds 30
    Assert-True ((Msg (Control 2100) 0x130B) -eq $Index) "Main tab $Index did not select"
}
function Choice([int]$Id,[int]$Index) {
    $h=Control $Id; Msg $h 0x130C $Index | Out-Null
    Msg ([TitleMenuSmoke.Native]::GetParent($h)) 0x111 ([uint64]($Id -bor 0x10000)) ($h.ToInt64()) | Out-Null
    Start-Sleep -Milliseconds 20
    Assert-True ((Msg $h 0x130B) -eq $Index) "Choice $Id selection did not persist"
}
function Theme([int]$Index) { Page 3; Choice 2039 5; Choice 2030 $Index; Start-Sleep -Milliseconds 40 }
function Offset { Guard; [TitleMenuSmoke.Native]::Offset((Control 2400),[uint32]$ownedPid) }
function Geometry([int]$Id,[int]$DocumentTop) {
    $h=Control $Id; $r=[TitleMenuSmoke.Native]::Outer($h,[uint32]$ownedPid); $p=[TitleMenuSmoke.Native]::Origin((Control 2400),[uint32]$ownedPid); $offset=Offset
    $actual=$r.Top-$p.Y; $expected=$DocumentTop-44-$offset
    $proof=[pscustomobject]@{Id=$Id;ActualY=$actual;ExpectedY=$expected;Scroll=$offset;Visible=[TitleMenuSmoke.Native]::IsWindowVisible($h);X=$r.Left-$p.X;Width=$r.Width;Height=$r.Height}
    Record 'HWND geometry' $proof
    Assert-True ($actual -eq $expected -and $proof.Visible) "Control $Id actual viewport Y=$actual expected=$expected"
}
function Resize([int]$Width,[int]$Height) { Guard; [UiRegression.Native]::Resize($window,[uint32]$ownedPid,$Width,$Height); Start-Sleep -Milliseconds 140 }
function Screen([string]$Name) {
    Guard; $path=Join-Path $output ($Name+'.png')
    try { $bounds=[UiRegression.Native]::Screen($window,[uint32]$ownedPid,$path); Record ('physical '+$Name) @{Path=$path;Bounds=$bounds}; return $path }
    catch { if($_.Exception.GetBaseException().Message -notlike 'SKIP physical capture:*') { throw }; Record ('SKIPPED physical '+$Name) $_.Exception.GetBaseException().Message; return $null }
}
function Residual([string]$Name,[switch]$Logs) {
    # No PrintWindow or forced repaint before the first physical image.
    $before=Screen ($Name+'-before'); $stable=if($Logs) { Text 2200 } else { '' }
    Guard; [UiRegression.Native]::Repaint((Control 2400),[uint32]$ownedPid); Start-Sleep -Milliseconds 160
    $after=Screen ($Name+'-after-485')
    if($null -eq $before -or $null -eq $after) { Record ('SKIPPED residual '+$Name) 'Physical capture blocked; no offscreen zero-difference acceptance'; return }
    $area=[UiRegression.Native]::Relative($window,(Control 2400),[uint32]$ownedPid)
    $exclude=[Collections.Generic.List[Drawing.Rectangle]]::new()
    if($Logs) {
        $exclude.Add([UiRegression.Native]::Relative($window,(Control 2220),[uint32]$ownedPid))
        if((Text 2200) -cne $stable) { Record ('SKIPPED residual '+$Name) 'Log content changed between captures; not stable'; return }
    }
    $proof=[UiRegression.Native]::Diff($before,$after,(Join-Path $output ($Name+'-diff.png')),$area,$exclude.ToArray())
    Record ('residual '+$Name) $proof
    Assert-True ($proof.Pixels -eq 0) "Persistent residual pixels in $Name after 0x485; inspect before/after/diff"
}
function Edit-State([IntPtr]$Handle) { Guard; [UiRegression.Native]::Edit($Handle,[uint32]$ownedPid) }
function All-EditRegions {
    $count=0
    foreach($h in [TitleMenuSmoke.Native]::Children($window,[uint32]$ownedPid,$true)) {
        if([TitleMenuSmoke.Native]::Class($h) -eq 'Edit') { Guard; [UiRegression.Native]::NoEditRegion($h,[uint32]$ownedPid); $count++ }
    }
    Record 'all native Edit HWNDs have no clipping region' $count
}
function Assert-Edits {
    All-EditRegions
    foreach($old in $editBaseline) {
        $now=Edit-State ([IntPtr]$old.Handle)
        foreach($key in @('Style','ExStyle','Selection','Margins','Mask','Limit','Length','Text','Password')) { Assert-True ($now.$key -ceq $old.$key) "Edit $($old.Handle) changed $key during presentation/navigation" }
    }
}
function Icon-Screenshot([string]$ThemeName) {
    $shot=Screen ($ThemeName+'-tabs')
    if($null -eq $shot) {
        Guard; $outer=Join-Path $output ($ThemeName+'-tabs-print-outer.png')
        [TitleMenuSmoke.Native]::Capture($window,[uint32]$ownedPid,$outer,$false) | Out-Null
        $r=[TitleMenuSmoke.Native]::Outer($window,[uint32]$ownedPid); $p=[TitleMenuSmoke.Native]::Origin($window,[uint32]$ownedPid); $c=[TitleMenuSmoke.Native]::Client($window,[uint32]$ownedPid)
        $shot=Join-Path $output ($ThemeName+'-tabs-OFFSCREEN.png')
        [UiRegression.Native]::Crop($outer,$shot,[Drawing.Rectangle]::new($p.X-$r.Left,$p.Y-$r.Top,$c.Width,$c.Height))
        Record ('SKIPPED physical icons '+$ThemeName) 'Using explicitly offscreen rendering proof only'
    }
    Guard
    $proof=[UiRegression.Native]::IconProof($window,(Control 2100),[uint32]$ownedPid,$shot,(Join-Path $project 'assets\navigation'),(Join-Path $output ($ThemeName+'-tab')),$ThemeName)
    Record ($ThemeName+' all four SVG mask/bounds proofs') $proof
    $urlEdits=@([TitleMenuSmoke.Native]::Children($window,[uint32]$ownedPid,$true) | Where-Object { [TitleMenuSmoke.Native]::Class($_) -eq 'Edit' -and [TitleMenuSmoke.Native]::GetDlgCtrlID($_) -eq 0 } | Where-Object { try { [TitleMenuSmoke.Native]::Text($_,[uint32]$ownedPid) -eq 'http://127.0.0.1:1/v1' } catch { $false } })
    Assert-True ($urlEdits.Count -eq 1) 'Synthetic native URL Edit missing'
    $editCrop=[UiRegression.Native]::Relative($window,$urlEdits[0],[uint32]$ownedPid)
    [UiRegression.Native]::Crop($shot,(Join-Path $output ($ThemeName+'-native-edit.png')),$editCrop)
    Record ($ThemeName+' native Edit border pixels') ([UiRegression.Native]::NativeBorder($shot,$editCrop))
    Guard; [TitleMenuSmoke.Native]::Capture($urlEdits[0],[uint32]$ownedPid,(Join-Path $output ($ThemeName+'-native-edit-OFFSCREEN.png')),$false) | Out-Null
}
No-Instance
$binary=Get-Item -LiteralPath $exe
$newest=Get-ChildItem (Join-Path $project 'src') -Recurse -File | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
if($newest.LastWriteTimeUtc -gt $binary.LastWriteTimeUtc) { throw 'Binary older than src; no rebuild or GUI run' }
[void][IO.Directory]::CreateDirectory($output)
$temp=Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-ui-regressions-'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temp)
$config=Join-Path $temp 'config.toml'
$toml="schema_version = 1`n[upstream]`nurl = 'http://127.0.0.1:1/v1'`n[listen]`naddress = '127.0.0.1'`nport = 4000`n[proxy]`nauto_start = false`nforward_enabled = false`nconnect_enabled = false`n[ui]`ntheme = 'light'`nremember_window_geometry = false`n[system]`nlogin_auto_start = false`n"
[IO.File]::WriteAllText($config,$toml,[Text.UTF8Encoding]::new($false)); $configHash=(Get-FileHash -LiteralPath $config).Hash
Record 'binary' @{Path=$exe;Utc=$binary.LastWriteTimeUtc.ToString('o');SHA256=(Get-FileHash -LiteralPath $exe).Hash;Freshness='all src timestamps only; no build'}
try {
    $oldDpi=[TitleMenuSmoke.Native]::EnterDpi()
    No-Instance
    $clock=[Diagnostics.Stopwatch]::StartNew()
    $child=Start-Process -FilePath $exe -WorkingDirectory $temp -ArgumentList @('--config-dir',('"'+$temp+'"')) -PassThru
    $childHandle=$child.Handle; $childStart=$child.StartTime; $ownedPid=$child.Id
    while($clock.Elapsed.TotalSeconds -lt 10) { Guard; $window=[TitleMenuSmoke.Native]::FindMain([uint32]$ownedPid); if($window -ne [IntPtr]::Zero) { break }; Start-Sleep -Milliseconds 50 }
    Assert-True ($window -ne [IntPtr]::Zero) 'Owned main HWND absent'
    $ready=$false
    while($clock.Elapsed.TotalSeconds -lt 15) {
        Guard; $start=[TitleMenuSmoke.Native]::FindControl($window,[uint32]$ownedPid,2102); $op=[TitleMenuSmoke.Native]::FindControl($window,[uint32]$ownedPid,2354)
        if($start -ne [IntPtr]::Zero -and $op -ne [IntPtr]::Zero -and [TitleMenuSmoke.Native]::IsWindowEnabled($start) -and [TitleMenuSmoke.Native]::Text($op,[uint32]$ownedPid).StartsWith('idle=1;')) { $ready=$true; break }; Start-Sleep -Milliseconds 60
    }
    Assert-True $ready 'Isolated fixture not ready'
    Record 'ownership' @{PID=$ownedPid;Start=$childStart.ToUniversalTime().ToString('o');Handle=$childHandle.ToInt64();Window=$window.ToInt64();Temp=$temp;DPI=[TitleMenuSmoke.Native]::GetDpiForWindow($window)}
    Assert-True ([TitleMenuSmoke.Native]::GetDpiForWindow($window) -eq 96) 'This expected-coordinate/mask run requires actual 96 DPI; no global DPI change'
    Resize 800 680
    try { Guard; [TitleMenuSmoke.Native]::Foreground($window,[uint32]$ownedPid); Record 'normal owned foreground request' 'accepted' } catch { Record 'foreground limitation' $_.Exception.GetBaseException().Message }
    $stage='first-visit logs'
    Page 2
    $logRect=[TitleMenuSmoke.Native]::Outer((Control 2200),[uint32]$ownedPid)
    Record 'first-visit log editor DIP height' $logRect.Height
    Assert-True ($logRect.Height -eq 240) 'First-visit logs editor must keep its default 240 DIP user-adjustable height, not stale pre-layout geometry'
    Screen 'light-logs-first-visit' | Out-Null
    $stage='history result/session coordinates'
    Page 1
    Guard; [TitleMenuSmoke.Native]::SetText((Control 2304),[uint32]$ownedPid,'127.0.0.2')
    Msg (Control 2304) 0xB1 1 5 | Out-Null
    $draft=Edit-State (Control 2304)
    Msg (Control 2400) 0x115 1 | Out-Null; Msg (Control 2400) 0x115 1 | Out-Null
    Assert-True ((Offset) -gt 0) 'History scroll did not move'
    for($cycle=0;$cycle -lt 3;$cycle++) {
        foreach($selection in @(2,3,0,1,2)) {
            Choice 2338 $selection
            $id=if($selection -eq 2) {2330} elseif($selection -eq 3) {2331} else {2337}
            Geometry $id 592; Geometry 2300 60
            $remembered=Offset; Page 0; Page 1
            Geometry $id 592; Assert-True ((Offset) -eq $remembered) 'History page return changed scroll'
        }
        Choice 2300 1; Geometry 2020 110; Geometry 2300 60
        Msg (Control 2400) 0x115 1 | Out-Null; Geometry 2020 110
        Choice 2300 0; Geometry 2330 592; Geometry 2300 60
        $now=Edit-State (Control 2304)
        Assert-True ($now.Text -ceq $draft.Text -and $now.Selection -eq $draft.Selection) 'History draft/selection lost during subview/scroll/page return'
    }
    Screen 'light-history-summary-scrolled' | Out-Null
    Choice 2301 4
    Record 'rounded date' ([UiRegression.Native]::Rounded((Control 2302),[uint32]$ownedPid))
    Msg (Control 2318) 0xF5 | Out-Null
    Record 'rounded combo' ([UiRegression.Native]::Rounded((Control 2310),[uint32]$ownedPid))
    Record 'rounded button' ([UiRegression.Native]::Rounded((Control 2320),[uint32]$ownedPid))
    Msg (Control 2318) 0xF5 | Out-Null
    $stage='native Edit baseline'
    Page 3; Choice 2039 0
    $editBaseline=[Collections.Generic.List[object]]::new(); $editBaseline.Add($draft)
    foreach($h in [TitleMenuSmoke.Native]::Children($window,[uint32]$ownedPid,$true)) {
        $id=[TitleMenuSmoke.Native]::GetDlgCtrlID($h)
        if([TitleMenuSmoke.Native]::Class($h) -eq 'Edit' -and ($id -eq 0 -or ($id -ge 2040 -and $id -le 2061))) { $editBaseline.Add((Edit-State $h)) }
    }
    Assert-True ($editBaseline.Count -ge 5 -and @($editBaseline | Where-Object Password).Count -ge 2) 'Settings/password Edit fixtures missing'
    foreach($e in $editBaseline) { if($e.Password) { Assert-True ($e.Length -eq 0 -and $e.Mask -ne 0) 'Expected empty masked synthetic password field' } }
    Record 'native Edit baselines (password text never read)' $editBaseline.ToArray()
    $stage='light icons'
    Icon-Screenshot 'light'
    $stage='dark category return residual'
    Theme 2; Choice 2039 0; Start-Sleep -Milliseconds 200
    Residual 'dark-settings-preview-return'
    Icon-Screenshot 'dark'
    $stage='dark active logs resize residual'
    Page 2
    foreach($size in @(@(960,740),@(800,560),@(900,700))) {
        Resize $size[0] $size[1]
        Residual ('dark-logs-'+$size[0]+'x'+$size[1]) -Logs
    }
    Page 3; Page 2; Residual 'dark-logs-cached-return' -Logs
    Assert-Edits
    $stage='theme/cache warmup and GDI stability'
    $cold=[TitleMenuSmoke.Native]::Resources($childHandle,[uint32]$ownedPid,0)
    for($warm=0;$warm -lt 4;$warm++) {
        Theme 1; foreach($p in @(0,1,2,3)) { Page $p }
        Theme 2; foreach($p in @(0,1,2,3)) { Page $p }
    }
    $warmCount=[TitleMenuSmoke.Native]::Resources($childHandle,[uint32]$ownedPid,0)
    $samples=[Collections.Generic.List[uint32]]::new()
    for($loop=0;$loop -lt 20;$loop++) {
        Theme 1; foreach($p in @(0,1,2,3)) { Page $p }
        Theme 2; foreach($p in @(0,1,2,3)) { Page $p }
        Start-Sleep -Milliseconds 60
                Guard; $samples.Add([TitleMenuSmoke.Native]::Resources($childHandle,[uint32]$ownedPid,0))
    }
    $range=$samples | Measure-Object -Minimum -Maximum
    Record 'GDI after full font/theme/page warmup' @{PreWarm=$cold;Warm=$warmCount;Samples=$samples.ToArray();ExpectedIconCache='8 bitmaps + 8 memory DCs maximum at one DPI, not a leak';Loops=20;WarmupLoops=4}
    Assert-True ($range.Maximum -eq $range.Minimum -and $range.Maximum -le $warmCount) 'GDI did not stabilize after warmup; investigate before calling a leak'
    Assert-Edits
    $stage='native directed keyboard/navigation'
    Page 1; Choice 2300 0
    # WM_NEXTDLGCTL is system-marshaled and scoped to this owned dialog tree.
    Msg $window 0x28 ([uint64](Control 2304).ToInt64()) 1 | Out-Null
    Guard; [TitleMenuSmoke.Native]::Key((Control 2304),[uint32]$ownedPid,0x27,$false,$false); Start-Sleep -Milliseconds 60
    $keyState=Edit-State (Control 2304)
    Assert-True ($keyState.Text -ceq $draft.Text -and $keyState.Selection -ne $draft.Selection) 'Native directed Right key did not preserve text and move selection'
    Msg (Control 2304) 0xB1 1 5 | Out-Null
    Msg $window 0x28 0 0 | Out-Null
    Guard; $focus=[UiRegression.Native]::Focus($window,[uint32]$ownedPid)
    Assert-True ($focus -ne [IntPtr]::Zero -and $focus -ne (Control 2304)) 'Native dialog next navigation did not move focus'
    Record 'keyboard scope' 'Owned WM_KEYDOWN/UP Right and WM_NEXTDLGCTL only; not physical SendInput/Tab/IME/accessibility acceptance'
    $stage='directory button truthful status'
    Page 0; Msg (Control 2106) 0xF5 | Out-Null; Start-Sleep -Milliseconds 700
    Guard
    $status=@([TitleMenuSmoke.Native]::Children($window,[uint32]$ownedPid,$false) | Where-Object { [TitleMenuSmoke.Native]::Class($_) -eq 'Static' } | ForEach-Object { [TitleMenuSmoke.Native]::Text($_,[uint32]$ownedPid) }) -join "`n"
    Record 'directory status' $status
    Assert-True ($status.Contains($temp) -and $status.Contains('不代表资源管理器窗口已显示')) 'Directory status lacks exact temp path or truthful ShellExecute caveat'
    # Read-only Explorer automation, bounded out of process; never input/close any
    # Explorer HWND. Reuse this script in query-only child mode is deliberately avoided.
    $query=[Diagnostics.ProcessStartInfo]::new()
    $query.FileName=Join-Path ([Environment]::GetFolderPath('System')) 'WindowsPowerShell\v1.0\powershell.exe'
    $query.UseShellExecute=$false; $query.RedirectStandardOutput=$true; $query.CreateNoWindow=$true
    $queryCode='$ErrorActionPreference="Stop";$s=New-Object -ComObject Shell.Application;try{@($s.Windows()|ForEach-Object{try{if($_.Document.Folder.Self.Path -eq '+"'"+$temp.Replace("'","''")+"'"+'){[pscustomobject]@{Path=$_.Document.Folder.Self.Path;HWND=$_.HWND}}}catch{}})|ConvertTo-Json -Compress}finally{[void][Runtime.InteropServices.Marshal]::ReleaseComObject($s)}'
    $encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($queryCode))
    $query.Arguments='-NoProfile -NonInteractive -EncodedCommand '+$encoded
    $reader=[Diagnostics.Process]::Start($query)
    if($reader.WaitForExit(2500)) {
            $metadata=$reader.StandardOutput.ReadToEnd()
            if($reader.ExitCode -eq 0 -and $metadata.Contains($temp.Replace('\','\\'))) { Record 'Explorer exact temp path (read-only query)' $metadata }
            else { Record 'SKIPPED Explorer window confirmation' 'Exact temp folder not observed in bounded read-only query; ShellExecute status alone is not window acceptance' }
            $reader.Dispose()
        }
    else { Record 'SKIPPED Explorer window confirmation' 'Read-only COM query exceeded 2.5s; not killed and no foreign HWND touched' }
    Screen 'dark-overview-directory-status' | Out-Null
    Record 'final stopped overview' (Text 2107)
    Assert-True ([TitleMenuSmoke.Native]::IsWindowEnabled((Control 2102)) -and -not [TitleMenuSmoke.Native]::IsWindowEnabled((Control 2103))) 'Proxy unexpectedly not stopped'
    Assert-True ((Get-FileHash -LiteralPath $config).Hash -eq $configHash) 'Config changed before exit'
    Record 'DPI acceptance limitation' 'Actual 96 DPI only. PMv2 harness context affects coordinate queries, not app DPI. No fake 150/200% or global DPI change.'
} catch { $failure=$_.Exception.GetBaseException().Message; Record ('STOP at '+$stage) $failure }
finally {
    if($null -ne $child -and -not $child.HasExited) {
        try {
            Identity
            if($window -eq [IntPtr]::Zero) { $window=[TitleMenuSmoke.Native]::FindMain([uint32]$ownedPid) }
            [TitleMenuSmoke.Native]::Post($window,[uint32]$ownedPid,0x111,1002,0)
            $normalExit=$child.WaitForExit(7000); Record 'owned command 1002 exit' $normalExit
        } catch { Record 'exit limitation (no kill/foreign input)' $_.Exception.GetBaseException().Message }
    }
    if(-not $normalExit -and -not $failure) { $failure='Owned normal exit was not confirmed' }
    $unchanged=(Get-FileHash -LiteralPath $config).Hash -eq $configHash
    Record 'config unchanged after exit' $unchanged
    if(-not $unchanged -and -not $failure) { $failure='Configuration changed' }
    if($null -ne $clock) { Record 'total owned GUI seconds' $clock.Elapsed.TotalSeconds }
    Record 'remaining ai-relay processes (query only)' @(Get-Process -Name 'ai-relay*' -ErrorAction SilentlyContinue | Select-Object Id,ProcessName)
        try { No-Instance; Record 'post-exit process and mutex absent' $true } catch { Record 'post-exit guard limitation' $_.Exception.GetBaseException().Message; if(-not $failure) { $failure='Post-exit process/mutex guard failed' } }
    if($oldDpi -ne [IntPtr]::Zero) { [void][TitleMenuSmoke.Native]::SetThreadDpiAwarenessContext($oldDpi) }
    $skips=@($events | Where-Object { $_.Name.StartsWith('SKIPPED') })
    [pscustomobject]@{Failure=$failure;Stage=$stage;NormalExit=$normalExit;Complete=($null -eq $failure -and $skips.Count -eq 0);Skipped=$skips;Events=$events.ToArray();TempRetained=$temp;NoBuild=$true;NoNetwork=$true;NoSave=$true;BudgetSeconds=$BudgetSeconds;InputScope='Pinned owned HWNDs only; main tabs native mouse messages; segmented choices scalar selection + normal WM_COMMAND; no remote pointer/WM_NOTIFY/IPC/global input';DpiScope='96 DPI only';ExplorerScope='One temp folder launch, read-only exact-path metadata, no closing Explorer'} | ConvertTo-Json -Depth 14 | Set-Content -LiteralPath (Join-Path $output 'report.json') -Encoding UTF8
    Write-Output ('REPORT '+(Join-Path $output 'report.json'))
}
if($failure) { throw $failure }
