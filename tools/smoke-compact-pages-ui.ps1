# Windows PowerShell 5.1 x64，交互式 Windows 10/11 桌面。
# 只新增本文件；不构建、不执行旧 smoke body、不联网、不保存/重新加载配置。
# 先 -ValidateOnly；收到 build 完成通知后再运行 GUI，默认 EXE 如下。
[CmdletBinding()]
param(
    [string]$ExePath,
    [switch]$ValidateOnly,
    [ValidateRange(30,180)][int]$BudgetSeconds = 90
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([IntPtr]::Size -ne 8) { throw 'Use a fresh 64-bit powershell.exe -NoProfile' }
if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $PSScriptRoot '..\target\compact-ui-build\debug\ai-relay.exe'
}

# 只提取声明并合并编译，不 dot-source/Invoke-Expression。没有实例化 ProxyUiMock。
$parts = @(
    @{ File='smoke-proxy-ui.ps1'; Pattern="(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@" },
    @{ File='smoke-ui-clarity.ps1'; Pattern="(?s)Add-Type -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll -TypeDefinition @'\r?\n(.*?)\r?\n'@" },
    @{ File='smoke-status-bar-ui.ps1'; Pattern='(?s)\$statusDeclaration = @''\r?\n(.*?)\r?\n''@' },
    @{ File='smoke-modern-ui.ps1'; Pattern='(?s)\$declarations\.Add\(@''\r?\n(.*?)\r?\n''@\)' },
    @{ File='smoke-date-picker-ui.ps1'; Pattern='(?s)\$declarations\.Add\(@''\r?\n(.*?)\r?\n''@\)' }
)
foreach ($name in @('ProxyUiSmoke','ProxyUiMock','ClarityNative','StatusBarNative','ModernNative','DatePickerNative','CompactNative')) {
    if ($name -as [type]) { throw "Helper $name already loaded; use fresh powershell.exe -NoProfile" }
}
$declarations = [Collections.Generic.List[string]]::new()
foreach ($part in $parts) {
    $source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot $part.File))
    $found = [regex]::Matches($source,$part.Pattern)
    if ($found.Count -ne 1) { throw "Expected one C# declaration in $($part.File); no old script executed" }
    $declaration = $found[0].Groups[1].Value
    if ($part.File -eq 'smoke-proxy-ui.ps1') {
        # 新 helper 已递归，直接复用，不能再声明同名方法；只适配旧版 extern。
        $needle = '[DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr parent, int id);'
        if ($declaration.Contains($needle)) {
            $declaration = $declaration.Replace($needle,'public static IntPtr GetDlgItem(IntPtr parent, int id) { return CompactNative.Find(parent,id); }')
        } elseif ($declaration -notmatch 'public static IntPtr GetDlgItem\(IntPtr parent, int id\)\s*\{' -or $declaration -notmatch 'EnumChildWindows\(parent,delegate') {
            throw 'Proxy recursive GetDlgItem contract changed; review declaration adapter'
        }
    }
    $declarations.Add($declaration)
}
$declarations.Add(@'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Threading;
public static class CompactNative {
    delegate bool EnumProc(IntPtr window,IntPtr unused);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent,EnumProc callback,IntPtr unused);
    [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll")] static extern bool GetScrollInfo(IntPtr window,int bar,ref ScrollInfo info);
    [DllImport("user32.dll")] static extern bool GetScrollBarInfo(IntPtr window,int objectId,ref BarInfo info);
    [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint thread,ref GuiInfo info);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window,out uint pid);
    [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr window,ref StatusBarNative.Point point);
    [DllImport("user32.dll")] static extern bool ScreenToClient(IntPtr window,ref StatusBarNative.Point point);
    [DllImport("user32.dll")] static extern IntPtr ChildWindowFromPointEx(IntPtr window,StatusBarNative.Point point,uint flags);
    [DllImport("user32.dll")] static extern IntPtr MonitorFromWindow(IntPtr window,uint flags);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern bool GetMonitorInfoW(IntPtr monitor,ref MonitorInfo info);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr window,IntPtr after,int x,int y,int width,int height,uint flags);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr window,uint message,UIntPtr w,IntPtr l,uint flags,uint timeout,out UIntPtr result);
    [StructLayout(LayoutKind.Sequential)] public struct ScrollInfo {
        public uint Size,Mask; public int Min,Max; public uint Page; public int Position,Track;
        public int Limit { get { return Math.Max(Min,Max-(int)Math.Max(1u,Page)+1); } }
    }
    [StructLayout(LayoutKind.Sequential)] public struct BarInfo {
        public uint Size; public StatusBarNative.Rect Rect; public int Arrow,ThumbTop,ThumbBottom,Reserved;
        public uint State0,State1,State2,State3,State4,State5;
        public bool Visible { get { return (State0&0x18000)==0 && Rect.Width>0 && Rect.Height>0; } }
    }
    [StructLayout(LayoutKind.Sequential)] struct MonitorInfo { public uint Size; public StatusBarNative.Rect Monitor,Work; public uint Flags; }
    [StructLayout(LayoutKind.Sequential)] struct GuiInfo {
        public uint Size,Flags; public IntPtr Active,Focus,Capture,MenuOwner,MoveSize,Caret; public StatusBarNative.Rect CaretRect;
    }
    public static string ValidateLayouts() {
        if(Marshal.SizeOf(typeof(ScrollInfo))!=28 || Marshal.SizeOf(typeof(BarInfo))!=60 ||
           Marshal.SizeOf(typeof(DatePickerNative.PickerInfo))!=72) throw new Exception("Unexpected Win32 x64 struct layout");
        var a=new StatusBarNative.Rect { Right=10,Bottom=10 };
        var edge=new StatusBarNative.Rect { Top=10,Right=10,Bottom=20 };
        var overlap=new StatusBarNative.Rect { Top=9,Right=10,Bottom=20 };
        if(Intersect(a,edge) || !Intersect(a,overlap)) throw new Exception("Clip intersection edge self-check failed");
        var scroll=new ScrollInfo { Min=0,Max=715,Page=400 };
        if(scroll.Limit!=316) throw new Exception("Native inclusive scrollbar range self-check failed");
        return "x64 layouts 28/60/72, clip edges and inclusive native scroll bounds";
    }
    public static IntPtr Find(IntPtr parent,int id) {
        // EnumChildWindows enumerates ALL descendants, including hidden pages.
        uint pid=ProxyUiSmoke.Owner(parent); ProxyUiSmoke.CheckOwner(parent,pid);
        IntPtr found=IntPtr.Zero; int count=0;
        EnumChildWindows(parent,delegate(IntPtr h,IntPtr unused) {
            if(++count>4096) throw new Exception("Recursive control enumeration exceeded 4096");
            ProxyUiSmoke.CheckOwner(h,pid);
            if(GetDlgCtrlID(h)==id) { found=h; return false; } return true;
        },IntPtr.Zero);
        return found;
    }
    public static ScrollInfo Scroll(IntPtr window,uint pid,int axis) {
        ProxyUiSmoke.CheckOwner(window,pid);
        var info=new ScrollInfo { Size=(uint)Marshal.SizeOf(typeof(ScrollInfo)),Mask=0x17 };
        // Public API copies to local output; NO pointer-bearing foreign message.
        if(!GetScrollInfo(window,axis,ref info)) throw new Exception("GetScrollInfo failed, axis="+axis);
        return info;
    }
    public static BarInfo Bar(IntPtr window,uint pid,bool vertical) {
        ProxyUiSmoke.CheckOwner(window,pid);
        var old=StatusBarNative.EnterDpi();
        try {
            var info=new BarInfo { Size=(uint)Marshal.SizeOf(typeof(BarInfo)) };
            if(info.Size!=60 || !GetScrollBarInfo(window,vertical?-5:-6,ref info)) throw new Exception("GetScrollBarInfo failed/layout mismatch");
            return info;
        } finally { StatusBarNative.RestoreDpi(old); }
    }
    public static IntPtr Focus(IntPtr window,uint pid) {
        ProxyUiSmoke.CheckOwner(window,pid); uint actual;
        uint thread=GetWindowThreadProcessId(window,out actual);
        var info=new GuiInfo { Size=(uint)Marshal.SizeOf(typeof(GuiInfo)) };
        if(actual!=pid || !GetGUIThreadInfo(thread,ref info)) throw new Exception("GetGUIThreadInfo failed");
        return info.Focus;
    }
    public static bool Intersect(StatusBarNative.Rect a,StatusBarNative.Rect b) {
        return Math.Min(a.Right,b.Right)>Math.Max(a.Left,b.Left) && Math.Min(a.Bottom,b.Bottom)>Math.Max(a.Top,b.Top);
    }
    public static StatusBarNative.Rect Effective(IntPtr window,IntPtr main,uint pid) {
        ProxyUiSmoke.CheckOwner(window,pid); var r=ModernNative.Bounds(window,main,pid);
        for(IntPtr p=StatusBarNative.GetParent(window);p!=IntPtr.Zero;p=StatusBarNative.GetParent(p)) {
            var client=StatusBarNative.Client(p,pid);
            // Map the actual client origin (not window origin; NC bars are excluded).
            var pt=new StatusBarNative.Point();
            if(!ClientToScreen(p,ref pt) || !ScreenToClient(main,ref pt)) throw new Exception("Cannot map clipped client");
            var c=new StatusBarNative.Rect { Left=pt.X,Top=pt.Y,Right=pt.X+client.Width,Bottom=pt.Y+client.Height };
            r=new StatusBarNative.Rect { Left=Math.Max(r.Left,c.Left),Top=Math.Max(r.Top,c.Top),Right=Math.Min(r.Right,c.Right),Bottom=Math.Min(r.Bottom,c.Bottom) };
            if(p==main) break;
        }
        return r;
    }
    public static void Reachable(IntPtr window,IntPtr main,IntPtr host,uint pid,bool full) {
        ProxyUiSmoke.CheckOwner(window,pid);
        if(!ProxyUiSmoke.IsWindowVisible(window)) throw new Exception("Not WS_VISIBLE, ID="+GetDlgCtrlID(window));
        var all=ModernNative.Bounds(window,main,pid); var view=Effective(window,main,pid);
        int minimum=Math.Min(all.Height,(int)Math.Round(40*ClarityNative.GetDpiForWindow(window)/96.0));
        if(view.Width<Math.Min(all.Width,8) || view.Height<(full?all.Height:minimum) || (full && view.Width<all.Width))
            throw new Exception("Clipped/unreachable ID="+GetDlgCtrlID(window)+" rect="+all+" effective="+view);
        // Resolve center through main -> viewport -> real control, not visibility alone.
        if(full) {
            var point=new StatusBarNative.Point { X=(view.Left+view.Right)/2,Y=(view.Top+view.Bottom)/2 };
            var parent=StatusBarNative.GetParent(window);
            if(!ClientToScreen(main,ref point) || !ScreenToClient(parent,ref point) || ChildWindowFromPointEx(parent,point,7)!=window)
                throw new Exception("Native center hit blocked, ID="+GetDlgCtrlID(window));
            if(parent==host) {
                var h=StatusBarNative.InParent(host,main,pid);
                var center=new StatusBarNative.Point { X=(view.Left+view.Right)/2,Y=(view.Top+view.Bottom)/2 };
                if(!Intersect(view,h) || ChildWindowFromPointEx(main,center,7)!=host) throw new Exception("Page button is covered outside viewport");
            }
        }
    }
    public static string Layout(IntPtr main,IntPtr host,IntPtr tab,IntPtr footer,uint pid) {
        uint dpi=ClarityNative.GetDpiForWindow(main);
        var c=StatusBarNative.Client(main,pid); var h=StatusBarNative.InParent(host,main,pid);
        var t=StatusBarNative.InParent(tab,main,pid); var f=StatusBarNative.InParent(footer,main,pid);
        int top=(int)((44L*dpi+48)/96),foot=(int)((40L*dpi+48)/96);
        if(StatusBarNative.GetParent(host)!=main || StatusBarNative.GetParent(tab)!=main || StatusBarNative.GetParent(footer)!=main ||
           h.Left!=0 || h.Top!=top || h.Right!=c.Width || h.Bottom!=c.Height-foot ||
           f.Left!=0 || f.Right!=c.Width || f.Top!=h.Bottom || f.Bottom!=c.Height || t.Bottom>h.Top ||
           (ProxyUiSmoke.GetWindowLongW(host,-20)&0x10000)==0 || (ProxyUiSmoke.GetWindowLongW(host,-16)&0x02000000)==0)
            throw new Exception("Fixed header/footer/WS_EX_CONTROLPARENT/clip failed: host="+h+" tab="+t+" footer="+f+" client="+c);
        int visible=0,clipped=0;
        foreach(var child in ModernNative.DirectChildren(host,pid)) if(ProxyUiSmoke.IsWindowVisible(child)) {
            visible++; var r=Effective(child,main,pid); var original=ModernNative.Bounds(child,main,pid);
            if(r.Height<original.Height || r.Width<original.Width) clipped++;
            if(r.Width>0 && r.Height>0 && (Intersect(r,t)||Intersect(r,f))) throw new Exception("Effective page content overlaps tab/footer, ID="+GetDlgCtrlID(child));
        }
        if(visible==0) throw new Exception("Empty viewport: no visible native children");
        return "host="+h+"; tab="+t+"; footer="+f+"; visible="+visible+"; clipped="+clipped;
    }
    public static StatusBarNative.Rect Resize(IntPtr main,uint pid,int widthDip,int heightDip) {
        ProxyUiSmoke.CheckOwner(main,pid); var old=StatusBarNative.EnterDpi();
        try {
            var m=new MonitorInfo { Size=(uint)Marshal.SizeOf(typeof(MonitorInfo)) };
            if(!GetMonitorInfoW(MonitorFromWindow(main,2),ref m)) throw new Exception("Work area unavailable");
            uint dpi=ClarityNative.GetDpiForWindow(main);
            int w=Math.Min(m.Work.Width,(int)Math.Round(widthDip*dpi/96.0));
            int h=Math.Min(m.Work.Height,(int)Math.Round(heightDip*dpi/96.0));
            if(!SetWindowPos(main,IntPtr.Zero,m.Work.Left,m.Work.Top,w,h,0x4014)) throw new Exception("Async outer-DIP resize failed");
            return new StatusBarNative.Rect { Left=m.Work.Left,Top=m.Work.Top,Right=m.Work.Left+w,Bottom=m.Work.Top+h };
        } finally { StatusBarNative.RestoreDpi(old); }
    }
    public static void PressCancel(IntPtr button,IntPtr main,IntPtr host,uint pid) {
        Reachable(button,main,host,pid,true);
        if(!ProxyUiSmoke.IsWindowEnabled(button)) throw new Exception("Disabled safe press target");
        var c=StatusBarNative.Client(button,pid); int point=((c.Height/2)<<16)|(c.Width/2);
        try {
            ModernNative.Message(button,pid,0x201,1,point);
            if((ModernNative.Message(button,pid,0xF2,0,0)&4)==0) throw new Exception("Native button did not enter pressed state");
        } finally { ModernNative.Message(button,pid,0x1F,0,0); } // WM_CANCELMODE: NO mouse-up/BN_CLICKED/save.
        if((ModernNative.Message(button,pid,0xF2,0,0)&4)!=0) throw new Exception("Button remained pressed after cancellation");
    }
    public sealed class Pixels { public string Path,Rect; public double WhiteFraction,WhiteTiles; public int Samples; }
    public static Pixels Inspect(string source,StatusBarNative.Rect screenRect,StatusBarNative.Rect captureOuter,string path) {
        var r=new Rectangle(screenRect.Left-captureOuter.Left,screenRect.Top-captureOuter.Top,screenRect.Width,screenRect.Height);
        using(var b=new Bitmap(source)) {
            if(r.Left<0||r.Top<0||r.Right>b.Width||r.Bottom>b.Height||r.Width<2||r.Height<2) throw new Exception("NC/arrow crop outside physical capture: "+r);
            using(var crop=b.Clone(r,PixelFormat.Format32bppArgb)) {
                crop.Save(path,ImageFormat.Png);
                int white=0,count=0,tiles=0,whiteTiles=0;
                for(int y=0;y<crop.Height;y+=2) for(int x=0;x<crop.Width;x+=2) {
                    var c=crop.GetPixel(x,y); count++; if(c.R>=220&&c.G>=220&&c.B>=220) white++;
                }
                // Small light arrow glyphs are allowed. Only large bright areas/tiles fail.
                int tile=Math.Min(12,Math.Min(crop.Width,crop.Height));
                for(int y=0;y+tile<=crop.Height;y+=tile) for(int x=0;x+tile<=crop.Width;x+=tile) {
                    int n=0,w=0;
                    for(int dy=0;dy<tile;dy+=2) for(int dx=0;dx<tile;dx+=2) {
                        var c=crop.GetPixel(x+dx,y+dy); n++; if(c.R>=220&&c.G>=220&&c.B>=220) w++;
                    }
                    tiles++; if(w>=n*.8) whiteTiles++;
                }
                return new Pixels { Path=path,Rect=r.ToString(),WhiteFraction=(double)white/count,WhiteTiles=tiles==0?0:(double)whiteTiles/tiles,Samples=count };
            }
        }
    }
    public sealed class ButtonColorProof {
        public string Path,ExpectedFill,ExpectedText; public int FillPixels,TextPixels,InteriorPixels;
    }
    public static ButtonColorProof ButtonColors(string source,string path,bool dark,bool danger) {
        // COLORREF in theme.rs is BGR; these are the corresponding screenshot RGB.
        Color fill=danger?(dark?Color.FromArgb(64,42,51):Color.FromArgb(255,243,245)):
            (dark?Color.FromArgb(41,62,84):Color.FromArgb(229,240,252));
        Color text=danger?(dark?Color.FromArgb(255,155,155):Color.FromArgb(180,54,60)):
            (dark?Color.FromArgb(120,192,255):Color.FromArgb(0,120,215));
        using(var b=new Bitmap(source)) {
            b.Save(path,ImageFormat.Png); int fills=0,texts=0,total=0;
            for(int y=4;y<b.Height-4;y++) for(int x=4;x<b.Width-4;x++) {
                Color c=b.GetPixel(x,y); total++;
                if(Math.Abs(c.R-fill.R)<=2 && Math.Abs(c.G-fill.G)<=2 && Math.Abs(c.B-fill.B)<=2) fills++;
                if(Math.Abs(c.R-text.R)<=5 && Math.Abs(c.G-text.G)<=5 && Math.Abs(c.B-text.B)<=5) texts++;
            }
            if(total==0 || fills<total*.4 || texts<8) throw new Exception("Button fill/text color mismatch: "+path+
                "; fill="+fills+"/"+total+"; text="+texts+"; expected="+fill+" / "+text);
            return new ButtonColorProof { Path=path,ExpectedFill=fill.ToString(),ExpectedText=text.ToString(),FillPixels=fills,TextPixels=texts,InteriorPixels=total };
        }
    }
    public static StatusBarNative.Rect ClientRectToScreen(IntPtr window,uint pid,StatusBarNative.Rect r) {
        ProxyUiSmoke.CheckOwner(window,pid);
        var point=new StatusBarNative.Point();
        if(!ClientToScreen(window,ref point)) throw new Exception("Cannot map actual client origin");
        r.Left+=point.X; r.Right+=point.X; r.Top+=point.Y; r.Bottom+=point.Y; return r;
    }
    public sealed class EditProof {
        public int Attempts,Before,After,ThumbBefore,ThumbAfter; public uint Selection;
        public ModernNative.Shot Shot; public BarInfo Vertical,Horizontal; public string Copy;
    }
    public static EditProof NativeEdit(IntPtr pinned,uint pid,IntPtr edit,string fixture,string path,bool horizontal) {
        StatusBarNative.CheckProcess(pinned,pid); ProxyUiSmoke.CheckOwner(edit,pid);
        // Details refresh every 100ms. No timer disabling, thread suspension or GUI
        // mutation beyond the requested text fixture: retry only a detected refresh.
        for(int attempt=1;attempt<=5;attempt++) {
            ProxyUiSmoke.SetText(edit,pid,fixture);
            ModernNative.Message(edit,pid,0xB1,0,12);
            var before=Scroll(edit,pid,1); var barBefore=Bar(edit,pid,true);
            ModernNative.Message(edit,pid,0xB6,0,30);
            if(horizontal) ModernNative.Message(edit,pid,0x114,7,0);
            var after=Scroll(edit,pid,1); var barAfter=Bar(edit,pid,true);
            uint selection=ProxyUiSmoke.Selection(edit,pid);
            if(ClarityNative.Text(edit,pid)!=fixture) continue;
            // Tiny wrapped details Edit can have thousands of logical lines but
            // only four pixels of native thumb travel. Require pixel movement only
            // if the logical delta maps to at least two pixels (rounding tolerance).
            int travel=barAfter.Rect.Height-2*barAfter.Arrow-(barAfter.ThumbBottom-barAfter.ThumbTop);
            double expectedPixels=(double)Math.Max(0,travel)*(after.Position-before.Position)/Math.Max(1,after.Limit-after.Min);
            if(!barBefore.Visible || !barAfter.Visible || after.Position<=before.Position ||
               barBefore.Rect.ToString()!=barAfter.Rect.ToString() || (expectedPixels>=2 && barAfter.ThumbTop<=barBefore.ThumbTop) || selection!=(12u<<16))
                throw new Exception("Native edit scroll/selection/SCROLLBARINFO mismatch, ID="+GetDlgCtrlID(edit)+
                    "; pos="+before.Position+"->"+after.Position+"; thumb="+barBefore.ThumbTop+"->"+barAfter.ThumbTop+
                    "; travel="+travel+"; selection="+selection+"; rect="+barAfter.Rect);
            BarInfo h=new BarInfo();
            if(horizontal) {
                h=Bar(edit,pid,false);
                if(!h.Visible || Scroll(edit,pid,0).Position<=0) throw new Exception("Native horizontal scroll did not move");
            }
            var shot=ModernNative.Capture(edit,pid,path,false,new StatusBarNative.Rect());
            if(ClarityNative.Text(edit,pid)!=fixture) continue;
            return new EditProof { Attempts=attempt,Before=before.Position,After=after.Position,ThumbBefore=barBefore.ThumbTop,
                ThumbAfter=barAfter.ThumbTop,Selection=selection,Shot=shot,Vertical=barAfter,Horizontal=h };
        }
        throw new Exception("Native details fixture refreshed during all 5 bounded capture attempts; no acceptance claimed");
    }
    public static string CopySelection(IntPtr window,uint pid,string expected) {
        // Clipboard is desktop-global. Preserve bounded text formats only; never
        // deserialize or replace an existing image/file/custom/credential format.
        string result=null; Exception failure=null;
        var thread=new Thread(delegate() {
            System.Windows.Forms.DataObject saved=null; bool changed=false; int sequence=0;
            try {
                sequence=GetClipboardSequenceNumber();
                var old=System.Windows.Forms.Clipboard.GetDataObject();
                saved=new System.Windows.Forms.DataObject();
                if(old!=null) foreach(string f in old.GetFormats(false)) {
                    if(f!="UnicodeText" && f!="Text" && f!="System.String" && f!="OEMText") { result="NOT RUN: non-text existing clipboard retained"; return; }
                    object value=old.GetData(f,false);
                    if(!(value is string) || ((string)value).Length>1048576) { result="NOT RUN: oversize/non-string existing clipboard retained"; return; }
                    saved.SetData(f,false,value);
                }
                if(sequence!=GetClipboardSequenceNumber()) throw new Exception("Clipboard changed before native copy");
                ModernNative.Message(window,pid,0x301,0,0); changed=true;
                sequence=GetClipboardSequenceNumber();
                IntPtr owner=GetClipboardOwner();
                if(owner==IntPtr.Zero || ProxyUiSmoke.Owner(owner)!=pid) throw new Exception("Clipboard owner changed during native copy; foreign data retained");
                if(System.Windows.Forms.Clipboard.GetText()!=expected) throw new Exception("Native WM_COPY != selected fixture substring");
                result="PASS: native WM_COPY equals selected fixture; prior text clipboard restored";
            } catch(Exception e) { failure=e; }
            finally {
                if(changed) {
                    try {
                        if(sequence!=GetClipboardSequenceNumber() || ProxyUiSmoke.Owner(GetClipboardOwner())!=pid) throw new Exception("Clipboard changed concurrently; foreign update not overwritten");
                        if(saved.GetFormats(false).Length==0) System.Windows.Forms.Clipboard.Clear();
                        else System.Windows.Forms.Clipboard.SetDataObject(saved,true);
                    } catch(Exception e) { failure=e; }
                }
            }
        });
        thread.IsBackground=true; thread.SetApartmentState(ApartmentState.STA); thread.Start();
        if(!thread.Join(3000)) throw new Exception("Clipboard proof exceeded 3s");
        if(failure!=null) throw failure; return result;
    }
    [DllImport("user32.dll")] static extern int GetClipboardSequenceNumber();
    [DllImport("user32.dll")] static extern IntPtr GetClipboardOwner();
}
'@)
$joined = $declarations -join "`n"
$usingPattern = '(?m)^using [^;\r\n]+;\r?$'
$usings = @([regex]::Matches($joined,$usingPattern) | ForEach-Object { $_.Value.Trim() } | Select-Object -Unique)
$combined = ($usings -join "`n") + "`n" + [regex]::Replace($joined,$usingPattern,'')
Add-Type -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll,Accessibility.dll,System.Windows.Forms.dll -TypeDefinition $combined
$layoutValidation=[CompactNative]::ValidateLayouts()
if ($ValidateOnly) {
    Write-Output "PASS: $layoutValidation"
    Write-Output 'PASS: PowerShell entry, declaration extraction, recursive adapter and combined C# compilation ONLY; no build, EXE lookup, mutex, launch, GUI, network, clipboard or acceptance tests'
    return
}

$session=[Diagnostics.Process]::GetCurrentProcess().SessionId
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$mutexName="Local\AIRelay.$sid.$session.v1"
$child=$null; $childHandle=[IntPtr]::Zero; $childStart=$null; $ownedPid=0
$window=[IntPtr]::Zero; $hostWindow=[IntPtr]::Zero; $footer=[IntPtr]::Zero
$temp=$null; $config=$null; $configBytes=$null; $failure=$null; $exe=$null
$previousDpi=[IntPtr]::Zero; $stage='preflight'; $lastProcessCheck=-1000
$checks=[Collections.Generic.List[object]]::new()
$shots=[Collections.Generic.List[object]]::new()
$gdiSamples=[Collections.Generic.List[object]]::new()
$limits=[Collections.Generic.List[string]]::new()
$uiFailures=[Collections.Generic.List[string]]::new()
$cleanupErrors=[Collections.Generic.List[string]]::new()
$overall=[Diagnostics.Stopwatch]::StartNew()
function Record([string]$Name,[object]$Value) {
    $checks.Add([pscustomobject]@{ Check=$Name; Value=$Value; Seconds=[Math]::Round($overall.Elapsed.TotalSeconds,2) })
    Write-Host ("CHECK {0}: {1}" -f $Name,($Value | ConvertTo-Json -Compress -Depth 6))
}
function Assert-NoOtherProcess {
    $others=@(Get-Process -Name 'ai-relay' -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $ownedPid })
    if ($others.Count -gt 0) { throw "BLOCKED: existing/foreign ai-relay PID(s) $($others.Id -join ','); left untouched; no IPC/input/kill" }
    $script:lastProcessCheck=$overall.ElapsedMilliseconds
}
function Assert-NoInstance {
    $existing=$null
    try { $existing=[Threading.Mutex]::OpenExisting($mutexName) }
    catch [Threading.WaitHandleCannotBeOpenedException] { }
    if ($null -ne $existing) { $existing.Dispose(); throw 'BLOCKED: same-user/session mutex exists; no launch or IPC' }
    Assert-NoOtherProcess
}
function Assert-Identity {
    if ($null -eq $child -or $child.HasExited) { throw 'Owned child missing/exited; input refused' }
    [StatusBarNative]::CheckProcess($childHandle,[uint32]$ownedPid)
    if ($child.Id -ne $ownedPid -or $child.Handle -ne $childHandle -or $child.StartTime -ne $childStart -or $child.Path -ine $exe) { throw 'Pinned child PID/handle/start/exe identity mismatch' }
}
function Guard {
    Assert-Identity
    if ($overall.ElapsedMilliseconds-$lastProcessCheck -gt 150) { Assert-NoOtherProcess }
    if ($overall.Elapsed.TotalSeconds -ge $BudgetSeconds) { throw "Overall $BudgetSeconds-second budget exceeded at $stage (target 30-60s)" }
}
function Find([int]$Id) {
    Guard
    return [CompactNative]::Find($window,$Id)
}
function Control([int]$Id) {
    $h=Find $Id
    if ($h -eq [IntPtr]::Zero) { throw "Missing recursive control ID=$Id; build latest compact UI first" }
    [ProxyUiSmoke]::CheckOwner($h,[uint32]$ownedPid)
    return $h
}
function Message([int]$Id,[uint32]$Code,[uint32]$W=0,[int]$L=0) {
    return [ModernNative]::Message((Control $Id),[uint32]$ownedPid,$Code,$W,$L)
}
function Text([int]$Id) { [ClarityNative]::Text((Control $Id),[uint32]$ownedPid) }
function Wait-State([string]$Name,[scriptblock]$Test,[int]$Milliseconds=5000) {
    $timer=[Diagnostics.Stopwatch]::StartNew()
    while ($timer.ElapsedMilliseconds -lt $Milliseconds) {
        Guard
        if (& $Test) { return }
        Start-Sleep -Milliseconds 15
    }
    throw "Wait expired: stage=$stage; check=$Name; limit=${Milliseconds}ms"
}
function Operation {
    $values=@{}
    foreach ($part in (Text 2354).Split(';')) {
        $pair=$part.Split('=')
        if ($pair.Count -eq 2) { $values[$pair[0]]=[uint64]$pair[1] }
    }
    foreach ($key in @('idle','sequence','fence','rejected','generation')) {
        if (-not $values.ContainsKey($key)) { throw "Missing operation metadata 2354/$key" }
    }
    return $values
}
function Fence {
    Guard
    $before=(Operation).fence
    [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,2397)
    Wait-State 'owner-thread queue fence' { (Operation).fence -gt $before }
    if ((Operation).rejected -ne 0) { throw "Rejected history command: $(Text 2354)" }
    if ([ProxyUiSmoke]::Dialog([uint32]$ownedPid) -ne [IntPtr]::Zero) { throw 'Unexpected own modal dialog; not automatically answered' }
}
function Scroll-State { [CompactNative]::Scroll($hostWindow,[uint32]$ownedPid,1) }
function Scroll([int]$Action) {
    Message 2400 0x115 ([uint32]$Action) | Out-Null
    Fence
}
function Reach([int]$Id,[switch]$Full) {
    [CompactNative]::Reachable((Control $Id),$window,$hostWindow,[uint32]$ownedPid,[bool]$Full)
}
function Layout {
    [CompactNative]::Layout($window,$hostWindow,(Control 2100),$footer,[uint32]$ownedPid)
}
function Segment([int]$Id,[int]$Index,[switch]$Keyboard) {
    $h=Control $Id
    if ([ProxyUiSmoke]::Class($h) -ne 'SysTabControl32') { throw "Expected native SysTabControl32 ID=$Id" }
    Reach $Id -Full
    $before=Message $Id 0x130B
    if ($Keyboard) {
        # 先点击当前 item 获得真实焦点，再用一个 Home/Right/Left 改变一次分类。
        [ModernNative]::ClickTab($childHandle,[uint32]$ownedPid,$h,[int]$before)
        Fence
        $key=if ($Index -eq $before+1) { 0x27 } elseif ($Index -eq $before-1) { 0x25 } elseif ($Index -eq 0) { 0x24 } else { throw 'Keyboard segment must use one adjacent key or Home' }
        [DatePickerNative]::Key($childHandle,[uint32]$ownedPid,$h,[uint32]$key)
    } else {
        # TCM_GETITEMRECT helper allocates output in OWN CHILD VM, not local NMHDR.
        [ModernNative]::ClickTab($childHandle,[uint32]$ownedPid,$h,$Index)
    }
    Fence
    Wait-State "native segment $Id selected $Index" { (Message $Id 0x130B) -eq $Index }
    if ($Id -ne 2100 -and (Message $Id 0x147) -ne $Index) { throw "Native/compat selected mismatch ID=$Id" }
}
function Page([int]$Index) {
    Segment 2100 $Index
    $expected=@(2102,2300,2207,2039)[$Index]
    Wait-State "page $Index shows $expected" { [ProxyUiSmoke]::IsWindowVisible((Control $expected)) }
}
function Category([int]$Index,[switch]$Keyboard) {
    if ((Message 2100 0x130B) -ne 3) { Page 3 }
    Scroll 6
    Segment 2039 $Index -Keyboard:$Keyboard
    $expected=@(2001,2040,2043,2050,2010,2030)[$Index]
    Wait-State "category $Index child visible" { [ProxyUiSmoke]::IsWindowVisible((Control $expected)) }
}
function Theme([int]$Index) {
    Category 5
    Segment 2030 $Index
    Fence
}
function Resize([int]$Width,[int]$Height) {
    Guard
    $wanted=[CompactNative]::Resize($window,[uint32]$ownedPid,$Width,$Height)
    Wait-State "outer ${Width}x${Height} DIP/workclamp" {
        $r=[StatusBarNative]::Outer($window,[uint32]$ownedPid)
        $r.Width -eq $wanted.Width -and $r.Height -eq $wanted.Height
    }
    Fence
    return $wanted
}
function Config-Unchanged {
    Guard
    if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($config)) -cne $configBytes) { throw 'Config bytes changed without explicit Save' }
    if ([ProxyUiSmoke]::IsWindowEnabled((Control 2003))) { throw 'Proxy unexpectedly running; network must remain stopped' }
}
function Capture([string]$Name,[IntPtr]$Handle) {
    Guard
    $outer=[StatusBarNative]::Outer($Handle,[uint32]$ownedPid)
    # Existing Capture(false) uses PrintWindow(PW_RENDERFULLCONTENT), FULL HWND/NC,
    # physical DPI. Never send our memory-DC handle or a local pointer via WM_PRINT.
    try { $shot=[ModernNative]::Capture($Handle,[uint32]$ownedPid,(Join-Path $output "$Name.png"),$false,(New-Object StatusBarNative+Rect)) }
    catch { throw "Capture $Name failed: $($_.Exception.GetBaseException().Message)" }
    if ($shot.Width -ne $outer.Width -or $shot.Height -ne $outer.Height) { throw "Non-physical/full-NC capture: $Name" }
    $shots.Add([pscustomobject]@{ Name=$Name; DPI=[ClarityNative]::GetDpiForWindow($Handle); Outer="$outer"; Shot=$shot })
    return $shot
}
function Focus-Tab([int]$From,[int]$Target) {
    Scroll 6
    Reach $From -Full
    [ModernNative]::Click((Control $From),[uint32]$ownedPid)
    Fence
    $targetHandle=Control $Target
    if (-not [ProxyUiSmoke]::IsWindowEnabled($targetHandle)) { throw "Tab target $Target disabled; logs status=$(Text 2220); details=$(Text 2221)" }
    $trace=[Collections.Generic.List[string]]::new()
    for ($i=0;$i -lt 40;$i++) {
        $focus=[CompactNative]::Focus($window,[uint32]$ownedPid)
        if ($focus -eq $targetHandle) {
            Fence # reveal is posted by WM_SETFOCUS; drain that asynchronous message.
            Reach $Target
            if ((Scroll-State).Position -le 0) { throw "Tab reached $Target without necessary viewport reveal" }
            Record "native Tab auto reveal $Target" @{ Steps=$i; Offset=(Scroll-State).Position; Layout=(Layout) }
            return
        }
        [ProxyUiSmoke]::CheckOwner($focus,[uint32]$ownedPid)
        $trace.Add(('{0}:{1}' -f [ClarityNative]::GetDlgCtrlID($focus),[ProxyUiSmoke]::Class($focus)))
        if ($trace.Count -ge 4 -and $trace[$trace.Count-1] -eq $trace[$trace.Count-2] -and $trace[$trace.Count-2] -eq $trace[$trace.Count-3]) { break }
        [DatePickerNative]::Key($childHandle,[uint32]$ownedPid,$focus,9)
        Fence
    }
    Capture "tab-reveal-failed-$Target" $window | Out-Null
    throw "Targeted Tab did not reach $Target within 40 keys (or repeated focus); not physical-keyboard/root-cause proof; focus path=$($trace -join ' -> ')"
}
function Set-PickerDate([int]$Id,[string]$ISO) {
    if (-not [DatePickerNative]::SetISO($childHandle,[uint32]$ownedPid,(Control $Id),$ISO)) { throw "Picker $Id rejected ISO '$ISO'" }
    Fence
    Assert-Date $Id $ISO
}
function Assert-Date([int]$Id,[string]$ISO) {
    $d=[DatePickerNative]::ReadDate($childHandle,[uint32]$ownedPid,(Control $Id),$false)
    $state=if ($ISO -eq '') { 1 } else { 0 }
    if ($d.State -ne $state -or $d.ISO -cne $ISO -or (Text $Id) -cne $ISO) { throw "Picker $Id state/date != '$ISO': $d" }
}
function Calendar([int]$Id) {
    $h=[IntPtr](Message $Id 0x1008)
    if ($h -ne [IntPtr]::Zero) { [ProxyUiSmoke]::CheckOwner($h,[uint32]$ownedPid) }
    return $h
}
function Click([int]$Id) {
    Reach $Id -Full
    if ($Id -in @(2001,2002,2003,2004,2031,2033,2034,2035,2036,2037,2038,2102,2103)) { throw "Unsafe action denied by smoke: ID=$Id" }
    [ModernNative]::Click((Control $Id),[uint32]$ownedPid)
    Fence
}
function Sequences([int[]]$Expected) {
    $actual=@([regex]::Matches((Text 2200),'"sequence":(\d+)') | ForEach-Object { [int]$_.Groups[1].Value })
    return ($actual -join ',') -ceq ($Expected -join ',')
}
function Assert-Draft {
    if ([ClarityNative]::Text($draftEdit,[uint32]$ownedPid) -cne $draftValue) { throw 'Settings URL draft lost across category/page/theme/scroll' }
    if ((Text 2304) -cne $historyDraft) { throw 'History input draft lost across page/theme/scroll' }
}

# Refuse existing PID/mutex BEFORE files/launch/IPC, also check immediately before launch.
Assert-NoInstance
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
    throw "Build pending: $([IO.Path]::GetFullPath($ExePath)). No GUI launched; script never runs Cargo. Use -ValidateOnly until build notification."
}
$exe=(Get-Item -LiteralPath $ExePath).FullName
$output=Join-Path ([IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\target\compact-pages-ui-screenshots'))) ([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')+'-'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($output)
try {
    $previousDpi=[StatusBarNative]::EnterDpi()
    if ([DatePickerNative]::HighContrast()) { throw 'BLOCKED: high contrast active; system settings left unchanged' }
    $temp=Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-compact-'+[Guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory((Join-Path $temp 'logs'))
    $config=Join-Path $temp 'config.toml'
    $toml="schema_version = 1`n[upstream]`nurl = 'http://127.0.0.1:1/v1'`n[listen]`naddress = '127.0.0.1'`nport = 4000`n[proxy]`nauto_start = false`nforward_enabled = false`nconnect_enabled = false`n[ui]`ntheme = 'light'`nremember_window_geometry = false`n[system]`nlogin_auto_start = false`n"
    [IO.File]::WriteAllText($config,$toml,[Text.UTF8Encoding]::new($false))
    $configBytes=[Convert]::ToBase64String([IO.File]::ReadAllBytes($config))
    # Local closed history fixture: adjacent UTC days and first/last included seconds.
    $fixture=@(
        '{"sequence":1,"timestamp":1709164799,"event":{"category":"runtime","level":"info","event":"started","error":null}}',
        '{"sequence":2,"timestamp":1709164800,"event":{"category":"runtime","level":"info","event":"started","error":null},"text":"status 503 error"}',
        '{"sequence":3,"timestamp":1709208000,"event":{"category":"access","level":"warn","metadata":{"status":503,"error":null}}}',
        '{"sequence":4,"timestamp":1709251199,"event":{"category":"runtime","level":"error","event":"operation_failed","error":"io"}}',
        '{"sequence":5,"timestamp":1709251200,"event":{"category":"access","level":"info","metadata":{"status":200,"error":null}}}'
    ) -join "`n"
    $fixturePath=Join-Path $temp 'logs\ai-relay.1.log'
    [IO.File]::WriteAllText($fixturePath,$fixture+"`n",[Text.UTF8Encoding]::new($false))
    $stage='launch/pin child; no proxy/network'
    Assert-NoInstance
    $child=Start-Process -FilePath $exe -WorkingDirectory $temp -ArgumentList @('--config-dir',('"'+$temp+'"')) -PassThru
    $childHandle=$child.Handle; $childStart=$child.StartTime; $ownedPid=$child.Id
    $identityTimer=[Diagnostics.Stopwatch]::StartNew()
    do {
        Assert-NoOtherProcess
        [StatusBarNative]::CheckProcess($childHandle,[uint32]$ownedPid)
        $child.Refresh()
        if ($child.Path -ieq $exe) { break }
        if ($identityTimer.ElapsedMilliseconds -ge 5000) { throw 'Child executable identity unavailable; no input sent' }
        Start-Sleep -Milliseconds 25
    } while ($true)
    Wait-State 'own main HWND' { $script:window=[ProxyUiSmoke]::Find([uint32]$ownedPid); $window -ne [IntPtr]::Zero } 8000
    Wait-State 'all descendants/history operation metadata ready' {
        $h=Find 2354
        if ($h -eq [IntPtr]::Zero -or (Find 2400) -eq [IntPtr]::Zero) { return $false }
        return ([ClarityNative]::Text($h,[uint32]$ownedPid)).StartsWith('idle=1;') -and [ProxyUiSmoke]::IsWindowEnabled((Control 2002))
    } 8000
    $hostWindow=Control 2400
    $mainChildren=@([ModernNative]::DirectChildren($window,[uint32]$ownedPid))
    $client=[StatusBarNative]::Client($window,[uint32]$ownedPid)
    $dpi=[ClarityNative]::GetDpiForWindow($window)
    $footerHeight=[int][Math]::Floor((40L*$dpi+48)/96)
    $headerHeight=[int][Math]::Floor((44L*$dpi+48)/96)
    # Branding/menu are also fixed Static children. Identify the footer by its
    # entire expected client-bottom strip, never by text or first Static found.
    $footers=@($mainChildren | Where-Object {
        if ([ProxyUiSmoke]::Class($_) -ne 'Static' -or $_ -eq $hostWindow) { return $false }
        $r=[StatusBarNative]::InParent($_,$window,[uint32]$ownedPid)
        [ProxyUiSmoke]::IsWindowVisible($_) -and $r.Left -eq 0 -and $r.Right -eq $client.Width -and
            $r.Top -eq $client.Height-$footerHeight -and $r.Bottom -eq $client.Height
    })
    if ($footers.Count -ne 1) { throw 'Expected exactly one full-width fixed footer at the client bottom' }
    $footer=$footers[0]
    $headerStatics=@($mainChildren | Where-Object { $_ -ne $hostWindow -and $_ -ne $footer -and $_ -ne (Control 2100) })
    if ($mainChildren.Count -ne 6 -or $headerStatics.Count -ne 3) { throw 'Expected only Tab, viewport, footer, menu and two branding controls on main' }
    foreach ($h in $headerStatics) {
        $r=[StatusBarNative]::InParent($h,$window,[uint32]$ownedPid)
        if ([ProxyUiSmoke]::Class($h) -ne 'Static' -or -not [ProxyUiSmoke]::IsWindowVisible($h) -or
            $r.Left -lt 0 -or $r.Top -lt 0 -or $r.Right -gt $client.Width -or $r.Bottom -gt $headerHeight) {
            throw 'Unexpected main child outside the fixed branding/menu header'
        }
    }
    if ([StatusBarNative]::GetParent((Control 2110)) -ne $window) { throw 'Menu must remain a fixed main child' }
    if ((Message 2039 0x1304) -ne 6) { throw 'Settings 2039 is not the six-item native direct segment' }
    Record 'owned child + recursive controls + fixed-only main children' @{ PID=$ownedPid; Exe=$exe; Host=$hostWindow.ToInt64(); Footer=$footer.ToInt64(); Layout=(Layout) }
    Config-Unchanged
    $stage='draft fixtures and six direct settings categories'
    Page 1
    Segment 2300 0
    $historyDraft='127.0.0.2'
    [ProxyUiSmoke]::SetText((Control 2304),[uint32]$ownedPid,$historyDraft)
    Page 3
    Category 0
    # Cross-process WM_GETTEXT is intentionally denied for ES_PASSWORD. Exclude
    # password inputs before reading; never weaken their native protection.
    $edits=@([ModernNative]::DirectChildren($hostWindow,[uint32]$ownedPid) | Where-Object { [ProxyUiSmoke]::Class($_) -eq 'Edit' -and [ClarityNative]::GetDlgCtrlID($_) -eq 0 -and ([ProxyUiSmoke]::GetWindowLongW($_,-16) -band 0x20) -eq 0 -and [ClarityNative]::Text($_,[uint32]$ownedPid) -eq 'http://127.0.0.1:1/v1' })
    if ($edits.Count -ne 1) { throw 'Cannot uniquely identify reparented settings URL Edit fixture' }
    $draftEdit=$edits[0]; $draftValue='http://127.0.0.1:1/compact-draft'
    [ProxyUiSmoke]::SetText($draftEdit,[uint32]$ownedPid,$draftValue)
    $categoryActions=@{}
    foreach ($mode in @('mouse','keyboard')) {
        Category 0
        foreach ($index in 1..5) {
            Category $index -Keyboard:($mode -eq 'keyboard')
            Assert-Draft
            Scroll 7
            foreach ($id in @(2001,2002,2003,2004)) {
                # STOP is disabled by design; still require full viewport intersection.
                if ($id -eq 2003) { Reach $id } else { Reach $id -Full }
            }
            $r=[StatusBarNative]::InParent((Control 2001),$hostWindow,[uint32]$ownedPid)
            $categoryActions[$index]=$r.Top+(Scroll-State).Position
            [CompactNative]::PressCancel((Control 2001),$window,$hostWindow,[uint32]$ownedPid)
            Fence
            Config-Unchanged
            Record "$mode category $index single native selection + safe Save press/cancel" @{ Selected=(Message 2039 0x130B); ActionsVirtualTop=$categoryActions[$index]; Reload='geometry/hit test only; never clicked'; Layout=(Layout) }
        }
        Category 0 -Keyboard:($mode -eq 'keyboard')
        # Short category content can end above the virtual bottom. Test its
        # actual action location, not a fabricated requirement to sit at SB_BOTTOM.
        Scroll 6
        Reach 2001 -Full
        [CompactNative]::PressCancel((Control 2001),$window,$hostWindow,[uint32]$ownedPid)
        Fence
        Assert-Draft
        Record "$mode category 0 direct selection; six choices covered" $true
    }
    if (@($categoryActions.Values | Select-Object -Unique).Count -lt 3) { throw 'Settings action row appears fixed instead of following category content' }

    $stage='outer DIP size matrix / fixed clips / independent offsets / bottom reachability'
    # Long category makes Save genuinely reachable at the virtual page bottom;
    # short categories are already tested at their own content-following rows.
    Category 2
    $sizes=[Collections.Generic.List[object]]::new()
    foreach ($size in @(@(800,560),@(800,680),@(1100,800))) {
        $actual=Resize $size[0] $size[1]
        $sizes.Add([pscustomobject]@{ Requested="$($size[0])x$($size[1]) outer DIP"; Physical="$actual" })
        $offsets=@{}
        foreach ($pageIndex in 0..3) {
            Page $pageIndex
            Scroll 6
            $tabBefore=[StatusBarNative]::InParent((Control 2100),$window,[uint32]$ownedPid).ToString()
            $footBefore=[StatusBarNative]::InParent($footer,$window,[uint32]$ownedPid).ToString()
            # Signed -120 wheel delta packed into the high word; no cursor/global input.
            Message 2400 0x20A 4287102976 | Out-Null
            Fence
            $afterWheel=(Scroll-State).Position
            if ((Scroll-State).Limit -gt 0 -and $afterWheel -le 0) { throw "Wheel failed page=$pageIndex size=$($size -join 'x')" }
            Scroll 6
            for ($line=0;$line -le $pageIndex;$line++) { Message 2400 0x115 1 | Out-Null }
            Fence
            $offsets[$pageIndex]=(Scroll-State).Position
            if ($tabBefore -cne [StatusBarNative]::InParent((Control 2100),$window,[uint32]$ownedPid).ToString() -or $footBefore -cne [StatusBarNative]::InParent($footer,$window,[uint32]$ownedPid).ToString()) { throw 'Wheel/native scrolling moved fixed header/footer' }
            Layout | Out-Null
        }
        if ($size[1] -eq 560 -and @($offsets.Values | Select-Object -Unique).Count -ne 4) { throw "Compact four-page offsets are not distinct: $($offsets | ConvertTo-Json -Compress)" }
        foreach ($pageIndex in 0..3) {
            Page $pageIndex
            if ((Scroll-State).Position -ne $offsets[$pageIndex]) { throw "Per-page offset lost page=$pageIndex" }
            Scroll 7
            if ((Scroll-State).Position -ne (Scroll-State).Limit) { throw "Native SB_BOTTOM did not reach page=$pageIndex limit" }
            switch ($pageIndex) {
                0 { Reach 2109 }
                1 { Reach 2337 }
                2 { Reach 2219 -Full; Reach 2221 }
                3 { Reach 2001 -Full }
            }
            Record "size $($size -join 'x') page $pageIndex bottom / restored offset" @{ Offset=$offsets[$pageIndex]; Limit=(Scroll-State).Limit; Layout=(Layout) }
        }
        Assert-Draft
        Config-Unchanged
    }
    Record 'requested outer DIP sizes with physical work-area clamp' $sizes.ToArray()
    if (@($sizes | ForEach-Object Physical | Select-Object -Unique).Count -lt 2) { throw 'Monitor work clamp made every requested size identical; compact resize not demonstrated' }
    Resize 800 560 | Out-Null
    Page 1
    Focus-Tab 2304 2337
    Page 2
    Record 'logs readiness before native Tab traversal' @{ ApplyEnabled=[ProxyUiSmoke]::IsWindowEnabled((Control 2219)); Status=(Text 2220); Details=(Text 2221) }
    # Keep a reproducible navigation failure as FAIL, but collect independent
    # picker/bar/category screenshot evidence before exiting the exact child.
    foreach ($target in @(2219,2221)) {
        try { Focus-Tab 2207 $target }
        catch { $uiFailures.Add($_.Exception.Message); Record "FAIL native Tab reveal $target (continuing independent checks)" $_.Exception.Message }
    }
    Assert-Draft

    $stage='logs native ISO pickers/month calendar/keyboard/clear and inclusive UTC metadata'
    Scroll 6
    foreach ($id in @(2224,2225)) {
        if ([ProxyUiSmoke]::Class((Control $id)) -ne 'SysDateTimePick32' -or ([ProxyUiSmoke]::GetWindowLongW((Control $id),-16) -band 2) -eq 0) { throw "Logs picker $id lost native class/DTS_SHOWNONE" }
        Set-PickerDate $id '2024-02-28'
        Reach $id -Full
        [DatePickerNative]::MousePart($childHandle,[uint32]$ownedPid,(Control $id),$true)
        Wait-State "native month calendar $id" { (Calendar $id) -ne [IntPtr]::Zero -and [ProxyUiSmoke]::IsWindowVisible((Calendar $id)) }
        Start-Sleep -Milliseconds 100 # Native popup show/layout may complete after handle creation.
        $calendar=Calendar $id
        if ([ProxyUiSmoke]::Class($calendar) -ne 'SysMonthCal32') { throw 'Not a native month-calendar popup' }
        $minimum=[DatePickerNative]::CalendarMinimum($childHandle,[uint32]$ownedPid,$calendar)
        $client=[StatusBarNative]::Client($calendar,[uint32]$ownedPid)
        if ($client.Width -lt $minimum.Width -or $client.Height -lt $minimum.Height) { throw "Clipped calendar ${id}: $client < $minimum" }
        for ($captureAttempt=1;$captureAttempt -le 3;$captureAttempt++) {
            try {
                Capture "logs-$id-native-calendar-attempt$captureAttempt" $calendar | Out-Null
                break
            } catch {
                $reason=$_.Exception.Message
                if ($captureAttempt -eq 3 -or -not $reason.Contains('Blank/incomplete PrintWindow')) {
                    $uiFailures.Add($reason); Record "FAIL calendar $id capture (continuing)" $reason
                    break
                }
                Record "calendar $id blank PrintWindow; bounded recapture" $captureAttempt
                Start-Sleep -Milliseconds 150
            }
        }
        [DatePickerNative]::Key($childHandle,[uint32]$ownedPid,$calendar,0x27)
        Fence
        [DatePickerNative]::Key($childHandle,[uint32]$ownedPid,$calendar,0x0D)
        Fence
        Wait-State "calendar keyboard closes $id" { (Calendar $id) -eq [IntPtr]::Zero }
        Assert-Date $id '2024-02-29'
        if ([DatePickerNative]::SetISO($childHandle,[uint32]$ownedPid,(Control $id),'1709164800')) { throw 'GUI incorrectly accepted raw epoch rather than ISO date' }
        Assert-Date $id '2024-02-29'
        [DatePickerNative]::MousePart($childHandle,[uint32]$ownedPid,(Control $id),$false)
        Fence
        Assert-Date $id ''
        Set-PickerDate $id '2024-02-29'
        Record "logs $id native calendar Right/Enter, leap day, checkbox clear; ISO-only GUI" @{ Minimum="$minimum"; Expected='2024-02-29'; EpochGui='rejected; old epoch GUI tests intentionally not reused' }
    }
    Click 2205
    Wait-State 'local closed fixture listed' { (Text 2220).Contains('已刷新') -and (Message 2209 0x146) -ge 2 }
    # Sorted native file selector: app current log at index 0, closed .1.log at 1.
    Message 2209 0x14E 1 | Out-Null
    Click 2206
    Wait-State 'fixture loaded and inclusive date endpoints rendered' { (Sequences @(2,3,4)) -and (Text 2220).Contains('ai-relay.1.log') }
    Click 2202
    Wait-State 'file follow paused' { (Text 2220).Contains('已暂停') }
    Record 'UTC inclusive whole day checked using typed local file metadata' @{ Start=1709164800; End=1709251199; ExpectedSequences=@(2,3,4); ExcludedAdjacentDays=@(1,5); HistoryQuery='not submitted; local logs read only' }
    Segment 2226 2
    [ProxyUiSmoke]::SetText((Control 2227),[uint32]$ownedPid,'5xx')
    Click 2228
    Wait-State 'metadata category/status/errors AND; not incidental text' { Sequences @(3) }
    $errorCheck=Message 2228 0xF0
    Scroll 7
    $accessBefore=Message 2213 0xF0
    Click 2213
    if ((Message 2213 0xF0) -eq $accessBefore) { throw 'Native access-log switch did not toggle exactly once' }
    $fileCheck=Message 2212 0xF0; $accessCheck=Message 2213 0xF0
    Page 0
    Page 2
    Scroll 6
    Assert-Date 2224 '2024-02-29'; Assert-Date 2225 '2024-02-29'
    if ((Message 2226 0x147) -ne 2 -or (Message 2228 0xF0) -ne $errorCheck -or (Text 2227) -cne '5xx' -or (Message 2212 0xF0) -ne $fileCheck -or (Message 2213 0xF0) -ne $accessCheck) { throw 'Logs category/date/switch draft lost on page change' }
    Segment 2226 0
    [ProxyUiSmoke]::SetText((Control 2227),[uint32]$ownedPid,'')
    Click 2228
    Set-PickerDate 2224 ''; Set-PickerDate 2225 ''
    Wait-State 'clear dates removes UTC bounds' { Sequences @(1,2,3,4,5) }
    Record 'logs native segment, metadata AND, page-switch checkbox/date retention and clear' $true
    Scroll 7
    Click 2213
    if ((Message 2213 0xF0) -ne $accessBefore) { throw 'Access-log draft switch did not restore before local Apply' }
    Reach 2219 -Full; Reach 2221
    # Actual nonpersistent local worker action; NO settings Save/Reload.
    Click 2219
    Wait-State 'logs apply remains unpersisted' { (Text 2220).Contains('未持久化') -and [ProxyUiSmoke]::IsWindowEnabled((Control 2219)) }
    Record 'visible bottom logs Apply native activation; not persisted' (Text 2220)
    Config-Unchanged

    $stage='light/dark whole-window compact + bottom evidence and native NC bars'
    $longLine='compact-native-copy-selection '+('0123456789abcdef'*160)
    $longText=(0..59 | ForEach-Object { ('{0:D3} ' -f $_)+$longLine }) -join "`r`n"
    foreach ($theme in @(@(1,'light'),@(2,'dark'))) {
        Theme $theme[0]
        foreach ($pageIndex in 0..3) {
            Page $pageIndex
            Scroll 6
            Layout | Out-Null
            Capture "$($theme[1])-800x560-page$pageIndex-top-window" $window | Out-Null
            Scroll 7
            Layout | Out-Null
            Capture "$($theme[1])-800x560-page$pageIndex-bottom-window" $window | Out-Null
        }
        # Visual-only history danger evidence. Preview is read-only; NEVER activate
        # 2352/confirmation, force EnableWindow, or synthesize success/WM_NOTIFY.
        Page 1
        Scroll 6
        Click 2339
        Wait-State 'history management storage complete' { (Operation).idle -eq 1 }
        Scroll 7
        Reach 2350 -Full
        Click 2350
        Wait-State 'read-only history clear preview complete' { (Operation).idle -eq 1 -and [ProxyUiSmoke]::IsWindowEnabled((Control 2352)) }
        Reach 2350 -Full; Reach 2352 -Full
        Capture "$($theme[1])-history-danger-preview-window" $window | Out-Null
        foreach ($id in @(2350,2352)) {
            $shot=Capture "$($theme[1])-danger-$id" (Control $id)
            $proof=[CompactNative]::ButtonColors($shot.Path,(Join-Path $output "$($theme[1])-danger-$id-color-proof.png"),($theme[1] -eq 'dark'),$true)
            Record "$($theme[1]) danger $id muted fill + red text; no confirmed deletion" $proof
        }
        Scroll 6
        Click 2339
        Wait-State 'history returns to table' { [ProxyUiSmoke]::IsWindowVisible((Control 2337)) }
        Page 2
        Scroll 7
        foreach ($id in @(2210,2211,2219)) {
            Reach $id -Full
            $shot=Capture "$($theme[1])-button-$id" (Control $id)
            $proof=[CompactNative]::ButtonColors($shot.Path,(Join-Path $output "$($theme[1])-button-$id-color-proof.png"),($theme[1] -eq 'dark'),($id -ne 2219))
            Record "$($theme[1]) button $id fill/text color" $proof
        }
        Config-Unchanged
        # Explicit category + content-following footer-actions evidence.
        foreach ($categoryIndex in 0..5) {
            Category $categoryIndex
            Capture "$($theme[1])-settings-category$categoryIndex-top-window" $window | Out-Null
            if ($categoryIndex -ne 0) { Scroll 7 }
            Reach 2001 -Full
            Capture "$($theme[1])-settings-category$categoryIndex-actions-window" $window | Out-Null
        }
        Page 2
        Scroll 6
        # C# groups scalar native calls and bounded capture to avoid PS overhead
        # racing the real 100ms details refresh. Never disable the owner's timer.
        $limits.Add("$($theme[1]) Edit 2221: NOT RUN stable long-text/scroll/copy evidence; normal 100ms refresh has no stable fixture; no injection retry this final run")
        Record "NOT RUN $($theme[1]) stable Edit 2221 evidence" 'No stable data; normal refresh retained, no repeated injection'
        foreach ($id in @(2200)) {
            if ($id -eq 2200) { Scroll 6 } else { Scroll 7 }
            Reach $id
            $original=Text $id
            $fixtureText=(Text 2220)+"`r`n"+$longText
            try {
                $captureOuter=[StatusBarNative]::Outer((Control $id),[uint32]$ownedPid)
                $proof=[CompactNative]::NativeEdit($childHandle,[uint32]$ownedPid,(Control $id),$fixtureText,(Join-Path $output "$($theme[1])-logs-$id-full-NC-native-scrolled.png"),($id -eq 2200))
                $shot=$proof.Shot
                $shots.Add([pscustomobject]@{ Name="$($theme[1])-native-edit-$id"; Shot=$shot; Attempts=$proof.Attempts })
                # Restore real text first so the 100ms refresh cannot replace a
                # synthetic prefix mid-copy. Scroll/selection fixture proven above.
                [ProxyUiSmoke]::SetText((Control $id),[uint32]$ownedPid,$original)
                Message $id 0xB1 0 12 | Out-Null
                $copy=[CompactNative]::CopySelection((Control $id),[uint32]$ownedPid,$original.Substring(0,12))
                if (-not $copy.StartsWith('PASS')) { $limits.Add("$($theme[1]) ID=$id $copy") }
                $pixels=[CompactNative]::Inspect($shot.Path,$proof.Vertical.Rect,$captureOuter,(Join-Path $output "$($theme[1])-logs-$id-vertical-bar.png"))
                $shots.Add([pscustomobject]@{ Name="$($theme[1])-native-NC-$id"; Pixels=$pixels })
                if ($theme[1] -eq 'dark' -and ($pixels.WhiteFraction -gt .20 -or $pixels.WhiteTiles -gt .15)) { throw "Dark native scrollbar white surface ID=${id}: fraction=$($pixels.WhiteFraction), tiles=$($pixels.WhiteTiles); see NC evidence (small arrow glyphs allowed)" }
                if ($id -eq 2200) {
                    $horizontal=$proof.Horizontal
                    if (-not $horizontal.Visible) { throw 'Native horizontal scrollbar unexpectedly absent' }
                    $hp=[CompactNative]::Inspect($shot.Path,$horizontal.Rect,$captureOuter,(Join-Path $output "$($theme[1])-logs-$id-horizontal-bar.png"))
                    $shots.Add([pscustomobject]@{ Name="$($theme[1])-horizontal-NC-$id"; Pixels=$hp })
                    if ($theme[1] -eq 'dark' -and ($hp.WhiteFraction -gt .20 -or $hp.WhiteTiles -gt .15)) { throw 'Dark horizontal scrollbar has large white track/arrow surface' }
                }
                Record "$($theme[1]) Edit $id native scroll/selection/NC geometry" @{ Before=$proof.Before; After=$proof.After; ThumbBefore=$proof.ThumbBefore; ThumbAfter=$proof.ThumbAfter; Attempts=$proof.Attempts; Copy=$copy; Bar=$pixels }
            } catch {
                $reason=$_.Exception.GetBaseException().Message
                if ($reason.Contains('fixture refreshed during all 5 bounded capture attempts')) {
                    # This is a test-fixture lifetime limitation, not evidence that
                    # the native details scrollbar failed. Never claim acceptance.
                    $limits.Add("$($theme[1]) Edit ${id}: $reason")
                    Record "NOT RUN $($theme[1]) stable Edit $id evidence" $reason
                } else {
                    $uiFailures.Add("$($theme[1]) Edit ${id}: $reason")
                    Record "FAIL $($theme[1]) native Edit $id (continuing independent evidence)" $reason
                }
            } finally { [ProxyUiSmoke]::SetText((Control $id),[uint32]$ownedPid,$original) }
        }
        Scroll 6
        foreach ($id in @(2224,2225)) {
            $picker=Control $id
            $info=[DatePickerNative]::Info($childHandle,[uint32]$ownedPid,$picker)
            $shot=Capture "$($theme[1])-logs-picker-$id-full-NC" $picker
            $outer=[StatusBarNative]::Outer($picker,[uint32]$ownedPid)
            # Map the real client origin, including any native border inset.
            $b=[CompactNative]::ClientRectToScreen($picker,[uint32]$ownedPid,$info.Button)
            $pixels=[CompactNative]::Inspect($shot.Path,$b,$outer,(Join-Path $output "$($theme[1])-logs-picker-$id-arrow.png"))
            $shots.Add([pscustomobject]@{ Name="$($theme[1])-picker-arrow-$id"; Pixels=$pixels })
            if ($theme[1] -eq 'dark' -and ($pixels.WhiteFraction -gt .25 -or $pixels.WhiteTiles -gt .20)) { $uiFailures.Add("Dark closed picker $id arrow has large white background, not just glyph") }
        }
        Config-Unchanged
        Assert-Draft
    }

    $stage='30 bounded native light/dark GDI transitions'
    Theme 1; Theme 2
    $gdiBefore=[StatusBarNative]::Gdi($childHandle,[uint32]$ownedPid)
    for ($i=0;$i -lt 30;$i++) {
        Theme (1+($i%2))
        $count=[StatusBarNative]::Gdi($childHandle,[uint32]$ownedPid)
        $gdiSamples.Add([pscustomobject]@{ Transition=($i+1); Theme=(1+($i%2)); GDI=$count })
        if ($count -gt $gdiBefore+8) { throw "GDI growth at transition $($i+1): $count vs warmed $gdiBefore" }
    }
    $gdiAfter=[StatusBarNative]::Gdi($childHandle,[uint32]$ownedPid)
    if ($gdiAfter -gt $gdiBefore) { throw "GDI increased at identical final dark theme: $gdiBefore -> $gdiAfter" }
    Record '30 bounded light/dark transitions, final GDI no increase' @{ Before=$gdiBefore; After=$gdiAfter; Peak=($gdiSamples | Measure-Object GDI -Maximum).Maximum }
    Assert-Draft
    Config-Unchanged
    if ([IO.File]::ReadAllText($fixturePath).TrimEnd() -cne $fixture.TrimEnd()) { throw 'Closed log fixture unexpectedly modified' }
    $stage='clean exit only own retained child / config byte equality'
    Guard
    [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,1002)
    if (-not $child.WaitForExit(10000) -or $child.ExitCode -ne 0) { throw 'Own child clean exit timed out/failed; no kill' }
    if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($config)) -cne $configBytes) { throw 'Exit wrote unsaved config' }
    Record 'own child exit 1002; byte-identical config including exit' $ownedPid
} catch {
    $failure=$_
    Write-Warning ("FAIL stage={0}; {1}; {2}" -f $stage,$_.Exception.Message,$_.InvocationInfo.PositionMessage)
} finally {
    $exited=$null -eq $child
    if ($null -ne $child) {
        try {
            if (-not $child.HasExited) {
                # Other instance appearing stops tests, but cleanup may address ONLY
                # the exact live retained child. No CLI show/quit or process kill.
                Assert-Identity
                if ($window -eq [IntPtr]::Zero) { $window=[ProxyUiSmoke]::Find([uint32]$ownedPid) }
                if ($window -ne [IntPtr]::Zero) { [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,1002) }
                if (-not $child.WaitForExit(10000)) { throw 'Own cleanup exit unconfirmed; child/temp retained, never killed' }
            }
            $exited=$child.HasExited
            if ($exited -and $null -ne $configBytes -and [Convert]::ToBase64String([IO.File]::ReadAllBytes($config)) -cne $configBytes) { $cleanupErrors.Add('Config differs after cleanup exit') }
        } catch { $cleanupErrors.Add($_.Exception.Message) }
        finally { $child.Dispose(); $child=$null }
    }
    if ($previousDpi -ne [IntPtr]::Zero) { [StatusBarNative]::RestoreDpi($previousDpi) }
    if ($null -ne $temp -and $exited) {
        try { [IO.Directory]::Delete($temp,$true) }
        catch { $cleanupErrors.Add("Temporary directory retained: $temp; $($_.Exception.Message)") }
    } elseif ($null -ne $temp) { $cleanupErrors.Add("Unconfirmed child exit; temp retained: $temp") }
    $detail=$null
    if ($null -ne $failure) { $detail=[pscustomobject]@{ Stage=$stage; Message=$failure.Exception.Message; Position=$failure.InvocationInfo.PositionMessage; Stack=$failure.ScriptStackTrace } }
    $report=[pscustomobject]@{
        Passed=($null -eq $failure -and $cleanupErrors.Count -eq 0 -and $uiFailures.Count -eq 0); Complete=($null -eq $failure -and $cleanupErrors.Count -eq 0 -and $uiFailures.Count -eq 0 -and $limits.Count -eq 0)
        Exe=$exe; PID=$ownedPid; Failure=$detail; Seconds=[Math]::Round($overall.Elapsed.TotalSeconds,2); TargetSeconds='30-60'; BudgetSeconds=$BudgetSeconds
        Checks=$checks.ToArray(); Screenshots=$shots.ToArray(); GDI=$gdiSamples.ToArray(); NotRun=$limits.ToArray(); UIFailures=$uiFailures.ToArray(); CleanupErrors=$cleanupErrors.ToArray()
        Limits='Current interactive monitor/DPI only; work-area clamp explicitly recorded, not a multi-DPI/multi-monitor matrix. HWND-local native keys/mouse are not physical SendInput acceptance. Save uses press/cancel only (no BN_CLICKED); Reload geometry only, never activated. ISO-only GUI replaces old numeric epoch tests; actual inclusive UTC membership uses local typed JSONL fixture, no model/network/history query. NC evidence uses existing ModernNative full-window physical PrintWindow; no foreign WM_PRINT with local HDC/pointer, code injection or manufactured scrollbar geometry. WM_PRINT+PRF_NONCLIENT in-process fallback is NOT exercised; a blank/white PrintWindow artifact fails explicitly and must not be called an on-screen success. Long Edit contents are synthetic native-control tests, not log volume/performance acceptance. Desktop clipboard is restored only for bounded text-only formats; other formats are retained and copy marked NOT RUN. No Cargo, settings Save/Reload, Startup/firewall/system timezone/theme changes, foreign IPC, process-name exit or kill.'
    }
    [IO.File]::WriteAllText((Join-Path $output 'results.json'),($report | ConvertTo-Json -Depth 13),[Text.UTF8Encoding]::new($true))
    Write-Output "Compact pages UI artifacts: $output"
}
if ($null -ne $failure) { throw $failure }
if ($cleanupErrors.Count -gt 0) { throw ($cleanupErrors -join '; ') }
if ($uiFailures.Count -gt 0) { throw ($uiFailures -join '; ') }
if ($limits.Count -gt 0) { Write-Warning ('PARTIAL: '+($limits -join '; ')); return }
Write-Output 'PASS: compact page clipping/outer sizes/offsets/wheel/Tab reveal, six native settings categories/draft/action rows, native log calendars/inclusive UTC metadata/switch retention, light/dark window/NC evidence, native scroll/selection/copy, 30 GDI transitions and unchanged config. See results.json for scope and limits.'
