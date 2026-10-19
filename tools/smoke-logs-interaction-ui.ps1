# Windows PowerShell 5.1 x64；仅主 agent 在独占的交互式桌面执行 GUI body。
# -ValidateOnly 仅解析/编译声明及纯逻辑自检；不查找 EXE、不创建 fixture、不启动 GUI。
# 不运行 Cargo，不执行被复用脚本的 body，不改真实配置/剪贴板/Startup/系统设置。
# 接受证据仅为按住阶段的真实 CopyFromScreen；遮挡不以 PrintWindow 替代。
[CmdletBinding()]
param(
    [string]$ExePath,
    [switch]$ValidateOnly,
    [ValidateRange(45,180)][int]$BudgetSeconds = 150
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([IntPtr]::Size -ne 8) { throw 'Use a fresh x64 powershell.exe -NoProfile process' }
foreach ($name in @('TitleMenuSmoke.Native','UiRegression.Native','LogsInteraction.Native')) {
    if ($name -as [type]) { throw "Helper $name already loaded; use a fresh PowerShell process" }
}
$project = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$declarations = [Collections.Generic.List[string]]::new()
foreach ($part in @(
    @{ File='smoke-title-menu-ui.ps1'; Pattern='(?s)\$nativeSource = @''\r?\n(.*?)\r?\n''@' },
    @{ File='smoke-ui-regressions.ps1'; Pattern='(?s)\$extra = @''\r?\n(.*?)\r?\n''@' }
)) {
    $source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot $part.File))
    $matchesFound = [regex]::Matches($source,$part.Pattern)
    if ($matchesFound.Count -ne 1) { throw "Expected exactly one declaration in $($part.File); no script body executed" }
    $declarations.Add($matchesFound[0].Groups[1].Value)
}
$interactionSource = @'
namespace LogsInteraction {
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using N = TitleMenuSmoke.Native;
public static class Native {
    [StructLayout(LayoutKind.Sequential)] public struct ScrollInfo {
        public uint Size,Mask; public int Min,Max; public uint Page; public int Position,Track;
        public int Limit { get { return Math.Max(Min,Max-(int)Math.Max(1u,Page)+1); } }
    }
    [StructLayout(LayoutKind.Sequential)] public struct BarInfo {
        public uint Size; public N.Rect Rect; public int Arrow,ThumbTop,ThumbBottom,Reserved;
        public uint State0,State1,State2,State3,State4,State5;
        public bool Visible { get { return (State0&0x18000)==0 && Rect.Width>0 && Rect.Height>0; } }
    }
    [StructLayout(LayoutKind.Sequential)] struct GuiInfo {
        public uint Size,Flags; public IntPtr Active,Focus,Capture,Menu,Move,Caret; public N.Rect CaretRect;
    }
    [StructLayout(LayoutKind.Sequential)] struct MonitorInfo { public uint Size; public N.Rect Monitor,Work; public uint Flags; }
    [StructLayout(LayoutKind.Sequential)] struct ContrastInfo { public uint Size,Flags; public IntPtr Scheme; }
    [DllImport("user32.dll")] static extern bool GetScrollInfo(IntPtr h,int axis,ref ScrollInfo info);
    [DllImport("user32.dll")] static extern bool GetScrollBarInfo(IntPtr h,int objectId,ref BarInfo info);
    [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint thread,ref GuiInfo info);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
    [DllImport("user32.dll")] static extern int GetWindowLongW(IntPtr h,int index);
    [DllImport("user32.dll")] static extern bool ScreenToClient(IntPtr h,ref N.Point point);
    [DllImport("user32.dll")] static extern uint MapVirtualKeyW(uint key,uint kind);
    [DllImport("user32.dll")] static extern IntPtr MonitorFromWindow(IntPtr h,uint flags);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern bool GetMonitorInfoW(IntPtr h,ref MonitorInfo info);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int w,int height,uint flags);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool QueryFullProcessImageNameW(IntPtr process,uint flags,StringBuilder text,ref int length);
    public const uint ResizeFlags = 0x4014; // ASYNCWINDOWPOS | NOZORDER | NOACTIVATE.
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern bool SystemParametersInfoW(uint action,uint size,ref ContrastInfo info,uint flags);
    [DllImport("user32.dll",CharSet=CharSet.Unicode,EntryPoint="SendMessageTimeoutW")]
    static extern IntPtr ReadMessage(IntPtr h,uint code,UIntPtr w,StringBuilder text,uint flags,uint timeout,out UIntPtr result);
    [DllImport("user32.dll",CharSet=CharSet.Unicode,EntryPoint="SendMessageTimeoutW")]
    static extern IntPtr StringMessage(IntPtr h,uint code,UIntPtr w,string text,uint flags,uint timeout,out UIntPtr result);
    public sealed class ViewerState {
        public long Handle,SemanticStyle,Selection,FirstLine; public int Length; public string TextSha256;
        public bool ReadOnly,Multiline; public ScrollInfo Vertical,Horizontal;
    }
    public sealed class LayoutProof {
        public int HeightDIP,EditorDocumentTop,GripGap,GripHeight,DocumentBottom,Extent,ExpectedExtent,Offset,TailGap;
    }
    public sealed class Pixels {
        public string Path,Region; public int Samples,BrightPixels,BrightTiles,Tiles,LongestBrightRun;
        public double WhiteFraction,WhiteTileFraction; public bool LargeWhite;
    }
    public static bool PathReady(string path,string expected) {
        if(String.IsNullOrWhiteSpace(path)) return false;
        if(!String.Equals(path,expected,StringComparison.OrdinalIgnoreCase)) throw new InvalidOperationException("Pinned executable metadata mismatch");
        return true;
    }
    public static void Image(IntPtr process,uint pid,string expected) {
        N.CheckProcess(process,pid);
        var path=new StringBuilder(32768); int length=path.Capacity;
        if(!QueryFullProcessImageNameW(process,0,path,ref length) || !PathReady(path.ToString(),expected))
            throw new InvalidOperationException("Pinned raw-handle image query failed");
    }
    public static bool ResizeCompleted(N.Rect actual,N.Rect requested) {
        return actual.Left==requested.Left && actual.Top==requested.Top && actual.Right==requested.Right && actual.Bottom==requested.Bottom;
    }
    public static int ClampHeight(int value) { return Math.Max(120,Math.Min(720,value)); }
    public static int DocumentBottom(int clientHeight,int height) { return Math.Max(656+height,clientHeight-56); }
    public static int Extent(int clientHeight,int height) { return Math.Max(clientHeight,656+height+40+16)-44-40; }
    public static long Style(IntPtr h,uint pid) { N.Check(h,pid); return unchecked((uint)GetWindowLongW(h,-16)); }
    public static void Tree(IntPtr main,IntPtr h,uint pid) {
        N.Check(main,pid);
        for(int i=0;i<64 && h!=IntPtr.Zero;i++,h=N.GetParent(h)) { N.Check(h,pid); if(h==main) return; }
        throw new Exception("HWND is not under the pinned owned main window");
    }
    public static bool HighContrast() {
        var info=new ContrastInfo { Size=(uint)Marshal.SizeOf(typeof(ContrastInfo)) };
        if(!SystemParametersInfoW(0x42,info.Size,ref info,0)) throw new Exception("Cannot query high contrast safely");
        return (info.Flags&1)!=0;
    }
    public static IntPtr CaptureOwner(IntPtr main,uint pid) {
        N.Check(main,pid); uint actual; uint thread=GetWindowThreadProcessId(main,out actual);
        var info=new GuiInfo { Size=(uint)Marshal.SizeOf(typeof(GuiInfo)) };
        if(actual!=pid || !GetGUIThreadInfo(thread,ref info)) throw new Exception("Owned GUI capture query failed");
        if(info.Capture!=IntPtr.Zero) Tree(main,info.Capture,pid);
        return info.Capture;
    }
    public static ScrollInfo Scroll(IntPtr h,uint pid,int axis) {
        N.Check(h,pid); if(axis!=0 && axis!=1) throw new Exception("Invalid scroll axis");
        var info=new ScrollInfo { Size=28,Mask=0x17 };
        if(!GetScrollInfo(h,axis,ref info)) throw new Exception("GetScrollInfo failed");
        return info;
    }
    public static BarInfo Bar(IntPtr h,uint pid,int axis) {
        N.Check(h,pid); if(axis!=0 && axis!=1) throw new Exception("Invalid bar axis");
        var info=new BarInfo { Size=60 };
        if(!GetScrollBarInfo(h,axis==1?-5:-6,ref info)) throw new Exception("GetScrollBarInfo failed");
        return info;
    }
    public static N.Point Center(IntPtr h,uint pid) {
        var r=N.Outer(h,pid); return new N.Point { X=r.Left+r.Width/2,Y=r.Top+r.Height/2 };
    }
    public static void ClientMouse(IntPtr h,uint pid,uint code,uint buttons,N.Point screen) {
        N.Check(h,pid);
        // Re-map the same SCREEN target after EVERY layout: the moving grip's old
        // client origin must never be reused. Captured moves may be outside client.
        if(!ScreenToClient(h,ref screen) || screen.X<short.MinValue || screen.X>short.MaxValue || screen.Y<short.MinValue || screen.Y>short.MaxValue)
            throw new Exception("Mouse client coordinate cannot be represented");
        N.Message(h,pid,code,buttons,N.Pack(screen));
    }
    public static void Key(IntPtr h,uint pid,uint key) {
        N.Check(h,pid); uint bits=1u|(MapVirtualKeyW(key,0)<<16);
        if(key>=0x21 && key<=0x28) bits|=0x01000000;
        N.Message(h,pid,0x100,key,unchecked((int)bits));
        N.Message(h,pid,0x101,key,unchecked((int)(bits|0xC0000000u)));
    }
    public static string Text(IntPtr h,uint pid) {
        N.Check(h,pid);
        if(N.Class(h)=="Edit" && (Style(h,pid)&0x20)!=0) throw new Exception("Password reads are forbidden");
        long length=N.Message(h,pid,14,0,0);
        if(length<0 || length>262144) throw new Exception("Text exceeds bounded 262144 UTF-16 fixture read");
        var text=new StringBuilder((int)length+1); UIntPtr copied;
        if(ReadMessage(h,13,new UIntPtr((uint)text.Capacity),text,0x22,1000,out copied)==IntPtr.Zero ||
           copied.ToUInt64()!=(ulong)length || text.Length!=length || N.Message(h,pid,14,0,0)!=length)
            throw new Exception("Bounded WM_GETTEXT timed out/truncated/changed");
        return text.ToString();
    }
    public static int ComboIndex(IntPtr combo,uint pid,string value) {
        N.Check(combo,pid); if(N.Class(combo)!="ComboBox") throw new Exception("Expected a native file ComboBox");
        UIntPtr result;
        // CB_FINDSTRINGEXACT is below WM_USER; user32 marshals this string.
        if(StringMessage(combo,0x158,new UIntPtr(ulong.MaxValue),value,0x22,1000,out result)==IntPtr.Zero)
            throw new Exception("CB_FINDSTRINGEXACT timed out");
        long index=unchecked((long)result.ToUInt64());
        if(index<0 || index>=N.Message(combo,pid,0x146,0,0) || index>=100) throw new Exception("Owned fixture absent from file list");
        return (int)index;
    }
    public static ViewerState Viewer(IntPtr edit,uint pid) {
        N.Check(edit,pid); if(N.Class(edit)!="Edit") throw new Exception("2200 must remain a native Edit");
        long style=Style(edit,pid); string text=Text(edit,pid); string hash;
        using(var sha=SHA256.Create()) hash=BitConverter.ToString(sha.ComputeHash(Encoding.UTF8.GetBytes(text))).Replace("-","");
        return new ViewerState { Handle=edit.ToInt64(),SemanticStyle=style&0x003108C4,ReadOnly=(style&0x800)!=0,Multiline=(style&4)!=0,
            Length=text.Length,TextSha256=hash,Selection=N.Message(edit,pid,0xB0,0,0),FirstLine=N.Message(edit,pid,0xCE,0,0),
            Vertical=Scroll(edit,pid,1),Horizontal=Scroll(edit,pid,0) };
    }
    public static N.Rect PostResize(IntPtr main,uint pid,int width,int height) {
        N.Check(main,pid); var monitor=new MonitorInfo { Size=(uint)Marshal.SizeOf(typeof(MonitorInfo)) };
        if(!GetMonitorInfoW(MonitorFromWindow(main,2),ref monitor)) throw new Exception("Work area query failed");
        width=Math.Min(width,monitor.Work.Width); height=Math.Min(height,monitor.Work.Height);
        if(width<800 || height<560) throw new Exception("SKIP fixture: work area cannot fit the 800x560 outer-pixel minimum");
        int x=monitor.Work.Left+Math.Min(20,monitor.Work.Width-width),y=monitor.Work.Top+Math.Min(20,monitor.Work.Height-height);
        if(!SetWindowPos(main,IntPtr.Zero,x,y,width,height,ResizeFlags)) throw new Exception("Owned async resize post failed");
        // A successful post is not completion; caller must observe this full rect.
        return new N.Rect { Left=x,Top=y,Right=x+width,Bottom=y+height };
    }
    public static bool LayoutSettled(IntPtr main,IntPtr host,IntPtr edit,IntPtr grip,uint pid,int height) {
        Tree(main,host,pid); Tree(main,edit,pid); Tree(main,grip,pid);
        var mc=N.Client(main,pid); var hc=N.Client(host,pid); var hp=N.Origin(host,pid);
        var hw=N.Outer(host,pid); var mp=N.Origin(main,pid);
        var e=N.Outer(edit,pid); var g=N.Outer(grip,pid); var s=Scroll(host,pid,1);
        return hw.Left==mp.X && hw.Top==mp.Y+44 && hw.Width==mc.Width && hw.Height==mc.Height-84 && hc.Height==mc.Height-84 && s.Page==(uint)hc.Height && s.Max+1==Extent(mc.Height,height) &&
            e.Top-hp.Y+44+s.Position==324 && e.Height==height && g.Top-e.Bottom==2 && g.Height==12 &&
            g.Left==e.Left && g.Right==e.Right && e.Left-hp.X==24 && e.Right-hp.X==hc.Width-24;
    }
    static int DocumentY(IntPtr h,IntPtr host,uint pid,int offset,bool bottom) {
        var r=N.Outer(h,pid); var p=N.Origin(host,pid); return (bottom?r.Bottom:r.Top)-p.Y+44+offset;
    }
    public static LayoutProof Layout(IntPtr main,IntPtr host,IntPtr edit,IntPtr grip,uint pid,int height) {
        if(height!=ClampHeight(height) || N.GetDpiForWindow(main)!=96) throw new Exception("Height/96-DPI contract invalid");
        Tree(main,host,pid); Tree(main,edit,pid); Tree(main,grip,pid);
        if(N.GetParent(edit)!=host || N.GetParent(grip)!=host) throw new Exception("Edit/grip must be direct viewport children");
        var e=N.Outer(edit,pid); var g=N.Outer(grip,pid); var hc=N.Client(host,pid); var hp=N.Origin(host,pid);
        var mc=N.Client(main,pid); var s=Scroll(host,pid,1); var tail=N.FindControl(main,pid,2221);
        N.Check(tail,pid); var t=N.Outer(tail,pid); var mp=N.Origin(main,pid);
        int top=DocumentY(edit,host,pid,s.Position,false),bottom=DocumentY(tail,host,pid,s.Position,true),expected=Extent(mc.Height,height);
        if(top!=324 || e.Height!=height || g.Top-e.Bottom!=2 || g.Height!=12 || g.Left!=e.Left || g.Right!=e.Right ||
           e.Left-hp.X!=24 || e.Right-hp.X!=hc.Width-24)
            throw new Exception("Viewer/grip geometry mismatch: top="+top+", height="+e.Height+"/"+height+", gap="+(g.Top-e.Bottom)+", grip="+g.Height+", left="+(e.Left-hp.X)+", right="+(e.Right-hp.X)+"/"+(hc.Width-24));
        if(bottom!=DocumentBottom(mc.Height,height) || s.Min!=0 || s.Position<0 || s.Position>s.Limit || s.Max+1!=expected || s.Page!=(uint)hc.Height || hc.Height!=mc.Height-84)
            throw new Exception("Log document bottom/scroll extent/page does not follow chosen height");
        int[] ids={2210,2211,2222,2223,2212,2213,2214,2215,2216,2217,2218,2219,2221};
        int[] oldY={432,432,432,432,538,538,612,612,612,612,612,656,698};
        for(int i=0;i<ids.Length;i++) {
            var h=N.FindControl(main,pid,ids[i]); N.Check(h,pid);
            if(N.GetParent(h)!=host || DocumentY(h,host,pid,s.Position,false)!=oldY[i]+height-96+8)
                throw new Exception("Lower log control did not follow height: ID="+ids[i]);
        }
        int gap=mp.Y+mc.Height-40-t.Bottom;
        if(s.Position==s.Limit && s.Limit>0 && gap!=16) throw new Exception("Scrolled log tail must finish 16 pixels above footer at 96 DPI");
        return new LayoutProof { HeightDIP=height,EditorDocumentTop=top,GripGap=g.Top-e.Bottom,GripHeight=g.Height,
            DocumentBottom=bottom,Extent=s.Max+1,ExpectedExtent=expected,Offset=s.Position,TailGap=gap };
    }
    public static int Travel(BarInfo bar,int axis) {
        return Math.Max(0,(axis==1?bar.Rect.Height:bar.Rect.Width)-2*bar.Arrow-(bar.ThumbBottom-bar.ThumbTop));
    }
    public static N.Point ThumbPoint(BarInfo bar,int axis,double fraction) {
        int grip=(bar.ThumbBottom-bar.ThumbTop)/2;
        int at=fraction<0?(bar.ThumbTop+bar.ThumbBottom)/2:bar.Arrow+(int)Math.Round(Travel(bar,axis)*fraction)+grip;
        return axis==1?new N.Point { X=(bar.Rect.Left+bar.Rect.Right)/2,Y=bar.Rect.Top+at }:
            new N.Point { X=bar.Rect.Left+at,Y=(bar.Rect.Top+bar.Rect.Bottom)/2 };
    }
    public static N.Point RepeatPoint(BarInfo bar,int axis,bool page) {
        int length=axis==1?bar.Rect.Height:bar.Rect.Width;
        int at=page?bar.Arrow+(int)((length-2*bar.Arrow)*.88):length-Math.Max(1,bar.Arrow/2);
        if(page && at<=bar.ThumbBottom) throw new Exception("Fixture has no page-after track at the requested point");
        return axis==1?new N.Point { X=(bar.Rect.Left+bar.Rect.Right)/2,Y=bar.Rect.Top+at }:
            new N.Point { X=bar.Rect.Left+at,Y=(bar.Rect.Top+bar.Rect.Bottom)/2 };
    }
    static bool White(int rgb) { return ((rgb>>16)&255)>=220 && ((rgb>>8)&255)>=220 && (rgb&255)>=220; }
    static Pixels Analyze(int[] colors,int width,int height,bool vertical) {
        var p=new Pixels { Samples=colors.Length }; int tile=Math.Min(8,Math.Min(width,height));
        foreach(int c in colors) if(White(c)) p.BrightPixels++;
        for(int y=0;y+tile<=height;y+=tile) for(int x=0;x+tile<=width;x+=tile) {
            int n=0; for(int dy=0;dy<tile;dy++) for(int dx=0;dx<tile;dx++) if(White(colors[(y+dy)*width+x+dx])) n++;
            p.Tiles++; if(n>=tile*tile*.8) p.BrightTiles++;
        }
        int cross=vertical?width:height,length=vertical?height:width;
        for(int a=0;a<cross;a++) { int run=0; for(int b=0;b<length;b++) {
            int rgb=vertical?colors[b*width+a]:colors[a*width+b];
            run=White(rgb)?run+1:0; p.LongestBrightRun=Math.Max(p.LongestBrightRun,run);
        } }
        p.WhiteFraction=(double)p.BrightPixels/p.Samples; p.WhiteTileFraction=p.Tiles==0?0:(double)p.BrightTiles/p.Tiles;
        p.LargeWhite=p.WhiteFraction>.20 || p.WhiteTileFraction>.15 || p.LongestBrightRun>=24;
        return p;
    }
    public static Pixels Inspect(string image,N.Rect bar,N.Point origin,string crop,bool vertical) {
        var rect=new Rectangle(bar.Left-origin.X,bar.Top-origin.Y,bar.Width,bar.Height);
        using(var bitmap=new Bitmap(image)) {
            if(rect.Width<2 || rect.Height<2 || !new Rectangle(Point.Empty,bitmap.Size).Contains(rect))
                throw new Exception("SKIP physical capture: complete non-client bar is outside owned client image");
            using(var region=bitmap.Clone(rect,PixelFormat.Format32bppArgb)) {
                var colors=new int[region.Width*region.Height];
                for(int y=0;y<region.Height;y++) for(int x=0;x<region.Width;x++) colors[y*region.Width+x]=region.GetPixel(x,y).ToArgb();
                var proof=Analyze(colors,region.Width,region.Height,vertical); region.Save(crop,ImageFormat.Png);
                proof.Path=crop; proof.Region=rect.ToString(); return proof;
            }
        }
    }
    public static string Validate() {
        if(IntPtr.Size!=8 || Marshal.SizeOf(typeof(ScrollInfo))!=28 || Marshal.SizeOf(typeof(BarInfo))!=60 || Marshal.SizeOf(typeof(GuiInfo))!=72)
            throw new Exception("Unexpected x64 SCROLLINFO/SCROLLBARINFO/GUITHREADINFO layout");
        if(ClampHeight(-100)!=120 || ClampHeight(240)!=240 || ClampHeight(1000)!=720 || Extent(641,240)!=868 || Extent(1600,120)!=1516)
            throw new Exception("Height clamp/document extent pure self-check failed");
        if(DocumentBottom(641,240)!=896 || DocumentBottom(952,240)!=896 || DocumentBottom(953,240)!=897 || DocumentBottom(1600,120)!=1544)
            throw new Exception("Detail tail must stretch only beyond the minimum log document bottom");
        if(new ScrollInfo { Min=0,Max=867,Page=557 }.Limit!=311) throw new Exception("Inclusive scroll range self-check failed");
        if((ResizeFlags&0x4000)==0 || (ResizeFlags&0x14)!=0x14) throw new Exception("Resize must post asynchronously without activation/z-order changes");
        var requested=new N.Rect { Left=-20,Top=30,Right=880,Bottom=770 };
        if(!ResizeCompleted(requested,requested) || ResizeCompleted(new N.Rect { Left=-20,Top=30,Right=880,Bottom=769 },requested) ||
           ResizeCompleted(new N.Rect { Left=-19,Top=30,Right=881,Bottom=770 },requested)) throw new Exception("Async resize completion self-check failed");
        if(PathReady(null,"expected.exe") || PathReady("  ","expected.exe") || !PathReady("EXPECTED.EXE","expected.exe")) throw new Exception("Process metadata readiness self-check failed");
        bool mismatch=false; try { PathReady("other.exe","expected.exe"); } catch(InvalidOperationException) { mismatch=true; }
        if(!mismatch) throw new Exception("Nonempty process metadata mismatch must fail closed");
        var bar=new BarInfo { Rect=new N.Rect { Left=-80,Top=100,Right=-63,Bottom=300 },Arrow=17,ThumbTop=17,ThumbBottom=57 };
        var start=ThumbPoint(bar,1,-1); var end=ThumbPoint(bar,1,1);
        if(start.X!=-71 || start.Y!=137 || end.Y!=263 || Travel(bar,1)!=126) throw new Exception("Screen/bar-relative thumb mapping self-check failed");
        var colors=new int[17*120]; for(int i=0;i<colors.Length;i++) colors[i]=0x151922;
        if(Analyze(colors,17,120,true).LargeWhite) throw new Exception("Dark pixel self-check failed");
        for(int y=2;y<5;y++) for(int x=2;x<5;x++) colors[y*17+x]=0xFFFFFF;
        if(Analyze(colors,17,120,true).LargeWhite) throw new Exception("Small glyph pixel self-check failed");
        for(int y=20;y<100;y++) colors[y*17+8]=0xFFFFFF;
        if(!Analyze(colors,17,120,true).LargeWhite) throw new Exception("Long narrow white stripe self-check failed");
        return "x64 layouts 28/60/72; async resize flags/completion; process Path pending/match/mismatch; 120..720 clamp, 240 default, dynamic extent, signed screen/thumb coordinates and dark/white pixel rules";
    }
}
}
'@
$declarations.Add($interactionSource)
Add-Type -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll,Accessibility.dll -TypeDefinition ($declarations -join "`n")
$tokens=$null; $parseErrors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($PSCommandPath,[ref]$tokens,[ref]$parseErrors)
if ($parseErrors.Count) { throw ('PowerShell parser errors: '+($parseErrors.Message -join '; ')) }
foreach ($typeName in @('TitleMenuSmoke.Native','UiRegression.Native','LogsInteraction.Native')) {
    $calls=$ast.FindAll({ param($n) $n -is [Management.Automation.Language.InvokeMemberExpressionAst] -and $n.Static -and $n.Expression.Extent.Text -eq "[$typeName]" },$true)
    foreach ($call in $calls) {
        if (@(([type]$typeName).GetMethods() | Where-Object { $_.IsStatic -and $_.Name -eq $call.Member.Value }).Count -eq 0) { throw "Missing public helper $typeName.$($call.Member.Value)" }
    }
}
$nativeValidation=[TitleMenuSmoke.Native]::Validate()
$interactionValidation=[LogsInteraction.Native]::Validate()
if ($ValidateOnly) {
    Write-Output "PASS: parser/static helper call sites, declaration-only extraction and combined C# compilation; $nativeValidation; $interactionValidation"
    Write-Output 'VALIDATION ONLY: no EXE lookup, process/mutex inspection, fixture/report creation, GUI, network, global input, clipboard or system mutation. No interaction/visual acceptance run.'
    return
}

if ([string]::IsNullOrWhiteSpace($ExePath)) { $ExePath=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\target\debug\ai-relay.exe')) }
if (-not [IO.Path]::IsPathRooted($ExePath)) { throw 'ExePath must be absolute; current directory is not a path base' }
$exe=[IO.Path]::GetFullPath($ExePath)
if ([IO.Path]::GetFileNameWithoutExtension($exe) -notlike 'ai-relay*') { throw 'Expected ai-relay* executable' }
$session=[Diagnostics.Process]::GetCurrentProcess().SessionId
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$mutexName="Local\AIRelay.$sid.$session.v1"
$child=$null; $childHandle=[IntPtr]::Zero; $childStart=$null; $ownedPid=0
$window=[IntPtr]::Zero; $hostWindow=[IntPtr]::Zero; $activeTarget=[IntPtr]::Zero; $oldDpi=[IntPtr]::Zero
$temp=$null; $config=$null; $fixturePath=$null; $output=$null; $clock=$null; $failure=$null; $normalExit=$false; $defaultRestored=$false
$matrixFinished=$false; $actualDpi=0
$configHash=$null; $fixtureHash=$null; $viewerBaseline=$null; $stage='preflight'
$events=[Collections.Generic.List[object]]::new(); $notRun=[Collections.Generic.List[string]]::new()
$frames=[Collections.Generic.List[object]]::new(); $pinnedControls=@{}; $heldPixels=@{}
function Record([string]$Name,[object]$Value) {
    $seconds=if ($null -eq $clock) { 0 } else { [Math]::Round($clock.Elapsed.TotalSeconds,3) }
    $events.Add([pscustomobject]@{Name=$Name;Seconds=$seconds;Value=$Value})
    Write-Host ($Name+': '+($Value | ConvertTo-Json -Depth 8 -Compress))
}
function Assert-True([bool]$Condition,[string]$Message) { if (-not $Condition) { throw ('REGRESSION: '+$Message) } }
function Limit([string]$Message) { if (-not $notRun.Contains($Message)) { $notRun.Add($Message); Record 'NOT RUN / incomplete' $Message } }
function No-Others {
    $others=@(Get-Process -Name 'ai-relay*' -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $ownedPid })
    if ($others.Count) { throw ('Foreign ai-relay* exists; no launch/input/IPC/kill: '+($others.Id -join ',')) }
}
function No-Instance {
    No-Others; $mutex=$null
    try { $mutex=[Threading.Mutex]::OpenExisting($mutexName) } catch [Threading.WaitHandleCannotBeOpenedException] {}
    if ($null -ne $mutex) { $mutex.Dispose(); throw 'Existing user/session mutex; launch refused' }
}
function Core-Identity {
    No-Others
    if ($null -eq $child -or $child.HasExited) { throw 'Pinned owned child missing/exited' }
    [LogsInteraction.Native]::Image($childHandle,[uint32]$ownedPid,$exe)
    if ($child.Id -ne $ownedPid -or $child.Handle -ne $childHandle -or $child.StartTime -ne $childStart) { throw 'Pinned PID/handle/start time changed' }
    if ($window -ne [IntPtr]::Zero) { [TitleMenuSmoke.Native]::Check($window,[uint32]$ownedPid) }
}
function Identity {
    Core-Identity
    if (-not [LogsInteraction.Native]::PathReady($child.Path,$exe)) { throw 'Pinned process metadata is not ready' }
}
function Wait-Identity([switch]$Cleanup) {
    $wait=[Diagnostics.Stopwatch]::StartNew()
    $maximum=if ($Cleanup) { 1000 } else { 5000 }
    $deadline=if ($Cleanup) { $BudgetSeconds-8 } else { $BudgetSeconds-12 }
    do {
        # Raw handle/image and creation identity are checked even while the
        # managed MainModule/Path metadata is empty. Never reopen a process by PID.
        Core-Identity
        $child.Refresh()
        Core-Identity
        if ([LogsInteraction.Native]::PathReady($child.Path,$exe)) { return }
        if ($wait.ElapsedMilliseconds -ge $maximum -or $clock.Elapsed.TotalSeconds -ge $deadline) {
            throw 'Bounded wait expired for pinned process Path metadata'
        }
        Start-Sleep -Milliseconds 40
    } while ($true)
}
function Guard {
    Identity
    if ($clock.Elapsed.TotalSeconds -ge ($BudgetSeconds-12)) { throw 'GUI budget reached; reserving 12 seconds for owned cleanup/command-1002 exit' }
}
function Control([int]$Id) {
    Guard
    $h=[TitleMenuSmoke.Native]::FindControl($window,[uint32]$ownedPid,$Id)
    [LogsInteraction.Native]::Tree($window,$h,[uint32]$ownedPid)
    if ($pinnedControls.ContainsKey($Id) -and $pinnedControls[$Id] -ne $h) { throw "Pinned HWND changed for control $Id" }
    $pinnedControls[$Id]=$h
    return $h
}
function Msg([IntPtr]$Handle,[uint32]$Code,[uint64]$W=0,[long]$L=0) {
    Guard; [LogsInteraction.Native]::Tree($window,$Handle,[uint32]$ownedPid)
    [TitleMenuSmoke.Native]::Message($Handle,[uint32]$ownedPid,$Code,$W,$L)
}
function Text([int]$Id) { [LogsInteraction.Native]::Text((Control $Id),[uint32]$ownedPid) }
function Capture-Owner { Guard; [LogsInteraction.Native]::CaptureOwner($window,[uint32]$ownedPid) }
function Wait-State([string]$Name,[scriptblock]$Test,[int]$Milliseconds=8000) {
    $wait=[Diagnostics.Stopwatch]::StartNew()
    while ($wait.ElapsedMilliseconds -lt $Milliseconds) { Guard; if (& $Test) { return }; Start-Sleep -Milliseconds 25 }
    throw "Bounded wait expired at $stage / $Name"
}
function Page([int]$Index) {
    $h=Control 2100
    [UiRegression.Native]::Tab($h,[uint32]$ownedPid,$Index)
    $expected=@(2109,2300,2200,2039)[$Index]
    Wait-State "page $Index visible" { (Msg $h 0x130B) -eq $Index -and [TitleMenuSmoke.Native]::IsWindowVisible((Control $expected)) }
}
function Choice([int]$Id,[int]$Index) {
    if ($Id -notin @(2039,2030)) { throw 'Only settings category/theme choices are allowed' }
    $h=Control $Id
    Assert-True ([TitleMenuSmoke.Native]::Class($h) -eq 'SysTabControl32') "Choice $Id must remain native SysTabControl32"
    Msg $h 0x130C ([uint64]$Index) | Out-Null
    Msg ([TitleMenuSmoke.Native]::GetParent($h)) 0x111 ([uint64]($Id -bor 0x10000)) ($h.ToInt64()) | Out-Null
    Wait-State "choice $Id index $Index" { (Msg $h 0x130B) -eq $Index }
}
function Theme([int]$Index) { Page 3; Choice 2039 5; Choice 2030 $Index; Start-Sleep -Milliseconds 100; Page 2 }
function Assert-Dark { Assert-True ((Msg (Control 2030) 0x130B) -eq 2) 'Expected application dark preference; no OS preference changes' }
function Scroll([int]$Action) { Msg (Control 2400) 0x115 ([uint64]$Action) | Out-Null }
function Reach([int]$Id) {
    $h=Control $Id
    Assert-True ([TitleMenuSmoke.Native]::IsWindowVisible($h) -and [TitleMenuSmoke.Native]::IsWindowEnabled($h)) "Input target $Id hidden/disabled"
    for ($i=0;$i -lt 40;$i++) {
        Guard; $r=[TitleMenuSmoke.Native]::Outer($h,[uint32]$ownedPid)
        $p=[TitleMenuSmoke.Native]::Origin($hostWindow,[uint32]$ownedPid); $c=[TitleMenuSmoke.Native]::Client($hostWindow,[uint32]$ownedPid)
        if ($r.Height -gt $c.Height -or $r.Left -lt $p.X -or $r.Right -gt $p.X+$c.Width) { throw "Fixture target $Id cannot fully fit actual viewport" }
        if ($r.Top -ge $p.Y -and $r.Bottom -le $p.Y+$c.Height) { return }
        $old=[TitleMenuSmoke.Native]::Offset($hostWindow,[uint32]$ownedPid)
        if ($r.Top -lt $p.Y) { Scroll 0 } else { Scroll 1 }
        if ([TitleMenuSmoke.Native]::Offset($hostWindow,[uint32]$ownedPid) -eq $old) { break }
    }
    throw "Control $Id not reachable by bounded native viewport scrolling"
}
function Click([int]$Id) {
    if ($Id -notin @(2205,2206,2202)) { throw "Action $Id is not allowlisted; no save/delete/open-directory actions" }
    Reach $Id; Msg (Control $Id) 0xF5 | Out-Null
}
function Resize([int]$Width,[int]$Height) {
    Guard; $before=[TitleMenuSmoke.Native]::Outer($window,[uint32]$ownedPid)
    $onLogs=(Msg (Control 2100) 0x130B) -eq 2
    $viewerHeight=if ($onLogs) { [TitleMenuSmoke.Native]::Outer((Control 2200),[uint32]$ownedPid).Height } else { 0 }
    $requested=[LogsInteraction.Native]::PostResize($window,[uint32]$ownedPid,$Width,$Height)
    Wait-State 'async resize full target rectangle observed' {
        [LogsInteraction.Native]::ResizeCompleted([TitleMenuSmoke.Native]::Outer($window,[uint32]$ownedPid),$requested)
    }
    Guard; $r=[TitleMenuSmoke.Native]::Outer($window,[uint32]$ownedPid)
    Assert-True ([LogsInteraction.Native]::ResizeCompleted($r,$requested)) 'Async resize changed before completion proof'
    if ($onLogs) {
        # The outer rect can be published before WM_SIZE has finished laying out
        # descendants. Wait for actual host/scroll/editor/grip geometry as well.
        Wait-State 'async resize log layout settled at selected height' {
            [LogsInteraction.Native]::LayoutSettled($window,(Control 2400),(Control 2200),(Control 2230),[uint32]$ownedPid,$viewerHeight)
        }
    }
    Record 'owned async resize completed (96-DPI outer pixels)' @{Requested="$Width x $Height";Target="$requested";Actual="$r";WorkAreaClamped=($r.Width -ne $Width -or $r.Height -ne $Height)}
    return ($before.Width -ne $r.Width -or $before.Height -ne $r.Height)
}
function Assert-Files {
    Assert-True ((Get-FileHash -LiteralPath $config).Hash -ceq $configHash) 'Fixture config.toml changed without Save'
    Assert-True ((Get-FileHash -LiteralPath $fixturePath).Hash -ceq $fixtureHash) 'Closed synthetic JSONL fixture changed'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $temp 'config.toml.bak'))) 'Unexpected config backup indicates implicit persistence'
}
function Assert-Viewer([string]$Name) {
    $h=Control 2200; [UiRegression.Native]::NoEditRegion($h,[uint32]$ownedPid)
    $now=[LogsInteraction.Native]::Viewer($h,[uint32]$ownedPid)
    foreach ($key in @('Handle','SemanticStyle','Length','TextSha256','Selection','ReadOnly','Multiline')) {
        Assert-True ($now.$key -ceq $viewerBaseline.$key) "$Name changed native viewer $key"
    }
    Assert-True ($now.ReadOnly -and $now.Multiline) "$Name lost native read-only multiline behavior"
}
function Assert-Height([int]$Height,[string]$Name) {
    $proof=[LogsInteraction.Native]::Layout($window,(Control 2400),(Control 2200),(Control 2230),[uint32]$ownedPid,$Height)
    if ($null -ne $viewerBaseline) { Assert-Viewer $Name }
    Record $Name $proof
}
function Physical([string]$Name,[IntPtr]$Held=[IntPtr]::Zero,[int]$Axis=-1,[string]$Case='',[switch]$AllowMotion) {
    Guard; $path=Join-Path $output ($Name+'.png'); $bar=$null; $scrollInfo=$null; $scrollAfter=$null
    if ($Held -ne [IntPtr]::Zero) { Assert-True ((Capture-Owner) -eq $Held) "$Name capture not owned before physical image" }
    if ($Axis -ge 0) {
        $bar=[LogsInteraction.Native]::Bar($Held,[uint32]$ownedPid,$Axis)
        $scrollInfo=[LogsInteraction.Native]::Scroll($Held,[uint32]$ownedPid,$Axis)
        Assert-True $bar.Visible "$Name native bar not visible"
    }
    try {
        $origin=[TitleMenuSmoke.Native]::Origin($window,[uint32]$ownedPid)
        $bounds=[UiRegression.Native]::Screen($window,[uint32]$ownedPid,$path)
        if ($Held -ne [IntPtr]::Zero) { Assert-True ((Capture-Owner) -eq $Held) "$Name released capture during physical image" }
        $pixels=$null
        if ($Axis -ge 0) {
            $after=[LogsInteraction.Native]::Bar($Held,[uint32]$ownedPid,$Axis)
            Assert-True ($after.Rect.ToString() -ceq $bar.Rect.ToString()) "$Name scrollbar strip moved while screenshotting"
            if (-not $AllowMotion) { Assert-True ($after.ThumbTop -eq $bar.ThumbTop -and $after.ThumbBottom -eq $bar.ThumbBottom) "$Name stationary thumb changed while screenshotting" }
            $scrollAfter=[LogsInteraction.Native]::Scroll($Held,[uint32]$ownedPid,$Axis)
            # Real repeat timers may move the thumb DURING CopyFromScreen. The
            # strip crop remains valid; record both positions, do not freeze timers.
            $pixels=[LogsInteraction.Native]::Inspect($path,$bar.Rect,$origin,(Join-Path $output ($Name+'-bar.png')),($Axis -eq 1))
        }
        $frame=[pscustomobject]@{Name=$Name;Path=$path;CaptureKind='Physical CopyFromScreen; NO PrintWindow/forced repaint';Held=($Held -ne [IntPtr]::Zero);CaptureHWND=$Held.ToInt64();Bounds=$bounds;ScrollBefore=$scrollInfo;ScrollAfter=$scrollAfter;Bar=$bar;Pixels=$pixels;Case=$Case;TimerMotionAllowed=[bool]$AllowMotion}
        $frames.Add($frame); Record 'physical frame' $frame
        if ($null -ne $pixels) {
            Assert-True (-not $pixels.LargeWhite) "$Name has a large bright NC surface during capture; inspect physical PNG/bar crop"
            if (-not $heldPixels.ContainsKey($Case)) { $heldPixels[$Case]=0 }; $heldPixels[$Case]++
        }
    } catch {
        $reason=$_.Exception.GetBaseException().Message
        if (-not $reason.StartsWith('SKIP physical capture:')) { throw }
        Limit ($Name+': '+$reason+'; no offscreen replacement')
    }
}
function Mouse([IntPtr]$Handle,[uint32]$Code,[uint32]$Buttons,[object]$Point) {
    Guard; [LogsInteraction.Native]::Tree($window,$Handle,[uint32]$ownedPid)
    [LogsInteraction.Native]::ClientMouse($Handle,[uint32]$ownedPid,$Code,$Buttons,$Point)
}
function Cancel-OwnedCapture {
    # Cleanup must remain usable after Guard's budget cut-off. Never release a
    # different process/thread's capture or kill a child after timeout.
    Identity
    $capture=[LogsInteraction.Native]::CaptureOwner($window,[uint32]$ownedPid)
    if ($capture -ne [IntPtr]::Zero) {
        [LogsInteraction.Native]::Tree($window,$capture,[uint32]$ownedPid)
        [TitleMenuSmoke.Native]::Message($capture,[uint32]$ownedPid,0x1F,0,0) | Out-Null
        Assert-True ([LogsInteraction.Native]::CaptureOwner($window,[uint32]$ownedPid) -eq [IntPtr]::Zero) 'Owned WM_CANCELMODE did not release capture'
    }
    $script:activeTarget=[IntPtr]::Zero
}
function Grip-Key([uint32]$Key,[int]$Height,[string]$Name,[int]$Times=1) {
    $before=[TitleMenuSmoke.Native]::Outer((Control 2200),[uint32]$ownedPid).Height
    # Begin-Grip reveals the 12-DIP target even at height 720, then its real
    # mouse-down sets focus. The main HWND is not a dialog focus-message host.
    $point=Begin-Grip; $h=Control 2230
    Release $h $point
    Assert-True ([UiRegression.Native]::Focus($window,[uint32]$ownedPid) -eq $h) 'Owned grip click must retain GUI-thread focus after mouse-up'
    Assert-True ([TitleMenuSmoke.Native]::Outer((Control 2200),[uint32]$ownedPid).Height -eq $before) 'No-motion grip focus click must not change viewer height'
    for ($i=0;$i -lt $Times;$i++) { Guard; [LogsInteraction.Native]::Key($h,[uint32]$ownedPid,$Key) }
    Assert-Height $Height $Name
}
function Begin-Grip {
    Reach 2230; $h=Control 2230
    Assert-True ((Capture-Owner) -eq [IntPtr]::Zero) 'Unexpected capture before grip down'
    $point=[LogsInteraction.Native]::Center($h,[uint32]$ownedPid); $script:activeTarget=$h
    Mouse $h 0x201 1 $point
    Wait-State 'grip owns capture before any move/up' { (Capture-Owner) -eq $h } 1500
    Assert-True ([UiRegression.Native]::Focus($window,[uint32]$ownedPid) -eq $h) 'Grip mouse-down must set GUI-thread focus while holding capture'
    return $point
}
function Grip-Move([object]$Start,[int]$Delta,[int]$Height,[string]$Name) {
    $h=Control 2230; $point=New-Object TitleMenuSmoke.Native+Point
    $point.X=$Start.X; $point.Y=$Start.Y+$Delta
    Mouse $h 0x200 1 $point
    Assert-True ((Capture-Owner) -eq $h) "$Name lost capture before release"
    Assert-Height $Height $Name
    Physical ($Name+'-HELD') $h
    return $point
}
function Release([IntPtr]$Handle,[object]$Point) {
    Mouse $Handle 0x202 0 $Point
    Wait-State 'capture released by actual owned mouse-up' { (Capture-Owner) -eq [IntPtr]::Zero } 1500
    $script:activeTarget=[IntPtr]::Zero
}
function Begin-NC([int]$Id,[int]$Axis,[object]$Point,[Diagnostics.Stopwatch]$DownClock=$null) {
    $h=Control $Id
    Assert-True ((Capture-Owner) -eq [IntPtr]::Zero) 'Unexpected capture before NC down'
    $hit=if ($Axis -eq 1) { 7 } else { 6 }
    $bar=[LogsInteraction.Native]::Bar($h,[uint32]$ownedPid,$Axis)
    $outer=[TitleMenuSmoke.Native]::Outer($h,[uint32]$ownedPid)
    $origin=[TitleMenuSmoke.Native]::Origin($h,[uint32]$ownedPid)
    $client=[TitleMenuSmoke.Native]::Client($h,[uint32]$ownedPid)
    $packed=[TitleMenuSmoke.Native]::Pack($Point)
    $actualHit=Msg $h 0x84 0 $packed
    # GetScrollBarInfo already returns SCREEN coordinates; do not add the
    # HWND/client origin again, or convert this WM_NCHITTEST lParam to client.
    Record 'NC pre-down native geometry and hit test' @{
        ID=$Id;Axis=$Axis;HWND=$h.ToInt64();Class=[TitleMenuSmoke.Native]::Class($h);DPI=[TitleMenuSmoke.Native]::GetDpiForWindow($h)
        Style=[LogsInteraction.Native]::Style($h,[uint32]$ownedPid);WindowScreenBounds=$outer
        ClientScreenOrigin=$origin;ClientBounds=$client;BarScreenOrigin=@{X=$bar.Rect.Left;Y=$bar.Rect.Top};Bar=$bar
        PointScreen=$Point;PointRelativeToBar=@{X=$Point.X-$bar.Rect.Left;Y=$Point.Y-$bar.Rect.Top}
        PointInsideBar=($Point.X -ge $bar.Rect.Left -and $Point.X -lt $bar.Rect.Right -and $Point.Y -ge $bar.Rect.Top -and $Point.Y -lt $bar.Rect.Bottom)
        NativeThumbCenterScreen=[LogsInteraction.Native]::ThumbPoint($bar,$Axis,-1)
        Scroll=[LogsInteraction.Native]::Scroll($h,[uint32]$ownedPid,$Axis);PackedScreenLParam=$packed
        ActualHitTest=$actualHit;ExpectedHitTest=$hit;ExpectedName=if($Axis -eq 1){'HTVSCROLL'}else{'HTHSCROLL'}
        CoordinateSource='SCROLLBARINFO.rcScrollBar screen rect + bar-relative thumb offsets; signed screen lParam, no origin addition'
    }
    Assert-True ($actualHit -eq $hit) "ID $Id native WM_NCHITTEST returned $actualHit; expected $hit (HTVSCROLL/HTHSCROLL); see NC pre-down geometry"
    $script:activeTarget=$h
    # Post, never synchronously wait on an old/native modal NC tracking loop.
    Guard
    if ($null -ne $DownClock) { $DownClock.Restart() }
    [TitleMenuSmoke.Native]::Post($h,[uint32]$ownedPid,0xA1,[uint64]$hit,[TitleMenuSmoke.Native]::Pack($Point))
    Wait-State "NC $Id/$Axis owns capture before move/up" { (Capture-Owner) -eq $h } 1500
}
function Prepare-NC([int]$Id,[int]$Axis) {
    if ($Id -eq 2200) { Reach 2200 }
    $h=Control $Id; $code=if ($Axis -eq 1) { 0x115 } else { 0x114 }
    Msg $h $code 6 | Out-Null
    if ($Id -eq 2200) { Reach 2200 }
    $bar=[LogsInteraction.Native]::Bar($h,[uint32]$ownedPid,$Axis)
    $info=[LogsInteraction.Native]::Scroll($h,[uint32]$ownedPid,$Axis)
    Assert-True ($bar.Visible -and $info.Limit -gt $info.Min -and [LogsInteraction.Native]::Travel($bar,$Axis) -ge 20) "ID $Id/$Axis needs a visible native scrollbar with meaningful thumb travel"
    return $bar
}
function Drag-NC([int]$Id,[int]$Axis) {
    $case="thumb-$Id-$Axis"; $bar=Prepare-NC $Id $Axis; $h=Control $Id
    $start=[LogsInteraction.Native]::ThumbPoint($bar,$Axis,-1); $last=$start
    $before=[LogsInteraction.Native]::Scroll($h,[uint32]$ownedPid,$Axis); $previous=$before.Position; $thumb=$bar.ThumbTop
    try {
        Begin-NC $Id $Axis $start
        Physical ($case+'-down-HELD') $h $Axis $case
        foreach ($fraction in @(.20,.50,.80)) {
            $last=[LogsInteraction.Native]::ThumbPoint($bar,$Axis,$fraction)
            Mouse $h 0x200 1 $last
            Assert-True ((Capture-Owner) -eq $h) "$case lost capture during move"
            $now=[LogsInteraction.Native]::Scroll($h,[uint32]$ownedPid,$Axis); $moved=[LogsInteraction.Native]::Bar($h,[uint32]$ownedPid,$Axis)
            Assert-True ($now.Position -gt $previous -and $now.Position -le $now.Limit -and $moved.ThumbTop -gt $thumb) "$case must move BOTH native position and native thumb before release"
            $suffix=([int]($fraction*100)).ToString('00')
            # Read-only native geometry first, then the physical frame BEFORE a
            # large text/selection read or diagnostics can delay the first sample.
            Physical ($case+'-'+$suffix+'-immediate-HELD') $h $Axis $case
            Start-Sleep -Milliseconds 80
            Physical ($case+'-'+$suffix+'-settled-HELD') $h $Axis $case
            Record ($case+' live native motion') @{Fraction=$fraction;Before=$previous;After=$now;ThumbBefore=$thumb;ThumbAfter=$moved.ThumbTop;Capture=$h.ToInt64();Released=$false}
            Assert-Viewer $case
            $previous=$now.Position; $thumb=$moved.ThumbTop
        }
        Release $h $last
        $after=[LogsInteraction.Native]::Scroll($h,[uint32]$ownedPid,$Axis)
        Assert-True ($after.Position -eq $previous) "$case changed native position only on release"
        Assert-Height 240 ($case+' released; body/selection/height preserved'); Assert-Dark
        Record ($case+' motion acceptance') @{NativePosition=$before.Position.ToString()+' -> '+$after.Position;HeldPixelFrames=if($heldPixels.ContainsKey($case)){$heldPixels[$case]}else{0};NoReleaseOnlyProof=$true}
        if (-not $heldPixels.ContainsKey($case) -or $heldPixels[$case] -lt 3) { Limit ($case+': fewer than three valid held physical bar frames') }
    } finally { Cancel-OwnedCapture }
}
function Repeat-NC([int]$Id,[int]$Axis,[switch]$PageTrack) {
    $part=if ($PageTrack) { 'page-track' } else { 'arrow' }; $case="$part-$Id-$Axis"
    $bar=Prepare-NC $Id $Axis; $h=Control $Id; $point=[LogsInteraction.Native]::RepeatPoint($bar,$Axis,[bool]$PageTrack)
    $downClock=[Diagnostics.Stopwatch]::new()
    try {
        Begin-NC $Id $Axis $point $downClock
        # SetCapture is observable before the posted down finishes its first step.
        # Neither capture nor a synchronous SendMessage is a posted-queue fence.
        # Require observation <200ms FROM POST, before the owner's 400ms repeat;
        # scheduling that misses this deadline fails, never accepts a later repeat.
        $firstStep=[pscustomobject]@{NativePosition=0;ElapsedFromPostedDownMs=0.0;DeadlineMs=200;OwnerRepeatDelayMs=400;ObservedBeforeDeadline=$false;QueueCompletionFence=$false}
        try {
            Wait-State "$case first native step before the repeat delay" {
                $firstStep.NativePosition=[LogsInteraction.Native]::Scroll($h,[uint32]$ownedPid,$Axis).Position
                $firstStep.ElapsedFromPostedDownMs=$downClock.Elapsed.TotalMilliseconds
                Assert-True ($firstStep.ElapsedFromPostedDownMs -lt $firstStep.DeadlineMs) "$case first native step not observed within 200ms of posted down; a later timer repeat cannot count as the first step"
                $firstStep.ObservedBeforeDeadline=$firstStep.NativePosition -gt 0
                return $firstStep.ObservedBeforeDeadline
            } 200
        } finally { Record ($case+' first native step timing (not a queue fence)') $firstStep }
        $initial=$firstStep.NativePosition
        Assert-True ($initial -gt 0 -and (Capture-Owner) -eq $h) "$case must perform its first native step while capture remains owned"
        Physical ($case+'-initial-HELD') $h $Axis $case -AllowMotion
        Wait-State "$case actual owner timer repeat (no synthetic WM_TIMER)" { [LogsInteraction.Native]::Scroll($h,[uint32]$ownedPid,$Axis).Position -gt $initial } 1800
        Assert-True ((Capture-Owner) -eq $h) "$case timer repeat lost capture"
        $repeated=[LogsInteraction.Native]::Scroll($h,[uint32]$ownedPid,$Axis).Position
        Physical ($case+'-timer-repeat-HELD') $h $Axis $case -AllowMotion
        Release $h $point
        $stopped=[LogsInteraction.Native]::Scroll($h,[uint32]$ownedPid,$Axis).Position
        Start-Sleep -Milliseconds 180
        Assert-True ([LogsInteraction.Native]::Scroll($h,[uint32]$ownedPid,$Axis).Position -eq $stopped) "$case kept repeating after release"
        Assert-Viewer $case; Assert-Dark
        Record ($case+' real timer acceptance') @{Initial=$initial;Repeated=$repeated;Stopped=$stopped;TimerMessagesInjected=$false}
        if (-not $heldPixels.ContainsKey($case) -or $heldPixels[$case] -lt 2) { Limit ($case+': missing held initial/repeat pixel evidence') }
    } finally { Cancel-OwnedCapture }
}

# Fail before creating files, launching a child or sending any IPC/input.
No-Instance
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Build unavailable: $exe; no GUI launched and this script never builds" }
$output=Join-Path (Join-Path $project 'target\logs-interaction-ui') ([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')+'-'+[Guid]::NewGuid().ToString('N').Substring(0,8))
[void][IO.Directory]::CreateDirectory($output)
$clock=[Diagnostics.Stopwatch]::StartNew()
try {
    $oldDpi=[TitleMenuSmoke.Native]::EnterDpi()
    if ([LogsInteraction.Native]::HighContrast()) { throw 'SKIP fixture: actual high contrast active; no OS settings changed' }
    $stage='private stopped fixture'
    $temp=Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-logs-interaction-'+[Guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory((Join-Path $temp 'logs'))
    $config=Join-Path $temp 'config.toml'; $fixturePath=Join-Path $temp 'logs\ai-relay.1.log'
    $toml="schema_version = 1`n[upstream]`nurl = 'http://127.0.0.1:1/v1'`n[listen]`naddress = '127.0.0.1'`nport = 4000`n[proxy]`nauto_start = false`nforward_enabled = false`nconnect_enabled = false`n[logging]`nlevel = 'off'`naccess_log = false`nfile_enabled = false`nretention_days = 7`nmax_file_bytes = 5242880`nretained_files = 3`nrecent_entries = 1000`n[ui]`ntheme = 'dark'`nremember_window_geometry = false`n[system]`nlogin_auto_start = false`n"
    [IO.File]::WriteAllText($config,$toml,[Text.UTF8Encoding]::new($false))
    $lines=[Collections.Generic.List[string]]::new(); $padding=[string]::new([char]'x',384)
    for ($i=0;$i -lt 256;$i++) {
        $lines.Add('{"sequence":'+$i+',"timestamp":1709251200,"event":{"category":"runtime","level":"info","event":"logs-interaction-fixture","error":null},"text":"fixture-row-'+$i.ToString('0000')+'-'+$padding+'"}')
    }
    $fixture=([string]::Join("`n",$lines)+"`n")
    Assert-True ([Text.Encoding]::UTF8.GetByteCount($fixture) -lt 192*1024) 'Fixture must leave room below the 256-KiB viewer bound'
    [IO.File]::WriteAllText($fixturePath,$fixture,[Text.UTF8Encoding]::new($false))
    $configHash=(Get-FileHash -LiteralPath $config).Hash; $fixtureHash=(Get-FileHash -LiteralPath $fixturePath).Hash
    Record 'isolated fixture (retained; no Save/network)' @{Temp=$temp;ConfigSHA256=$configHash;JSONLSHA256=$fixtureHash;Rows=256;FileBytes=(Get-Item -LiteralPath $fixturePath).Length;FileWriterEnabled=$false;Autostart=$false;RememberGeometry=$false;Exe=$exe;ExeSHA256=(Get-FileHash -LiteralPath $exe).Hash}
    No-Instance
    $child=Start-Process -FilePath $exe -WorkingDirectory $temp -ArgumentList @('--config-dir',('"'+$temp+'"')) -PassThru
    $childHandle=$child.Handle; $childStart=$child.StartTime; $ownedPid=$child.Id
    Wait-Identity
    Wait-State 'owned main HWND' { $script:window=[TitleMenuSmoke.Native]::FindMain([uint32]$ownedPid); $window -ne [IntPtr]::Zero }
    Wait-State 'loaded and stopped controller' {
        $start=[TitleMenuSmoke.Native]::FindControl($window,[uint32]$ownedPid,2102)
        $stop=[TitleMenuSmoke.Native]::FindControl($window,[uint32]$ownedPid,2103)
        $operation=[TitleMenuSmoke.Native]::FindControl($window,[uint32]$ownedPid,2354)
        # Startup status can change between GETTEXTLENGTH/GETTEXT. Read its bounded
        # prefix in one marshalled call; strict stable body reads remain unchanged.
        $start -ne [IntPtr]::Zero -and $stop -ne [IntPtr]::Zero -and $operation -ne [IntPtr]::Zero -and
            [TitleMenuSmoke.Native]::IsWindowEnabled($start) -and -not [TitleMenuSmoke.Native]::IsWindowEnabled($stop) -and
            [TitleMenuSmoke.Native]::Text($operation,[uint32]$ownedPid).StartsWith('idle=1;')
    }
    $actualDpi=[TitleMenuSmoke.Native]::GetDpiForWindow($window)
    Record 'actual application DPI (not harness awareness)' $actualDpi
    if ($actualDpi -ne 96) { throw 'SKIP fixture: this script requires actual 96 DPI; no fake DPI/global preference change' }
    $hostWindow=Control 2400
    Record 'pinned child' @{PID=$ownedPid;Handle=$childHandle.ToInt64();Start=$childStart.ToUniversalTime().ToString('o');Window=$window.ToInt64();DPI=96;BudgetSeconds=$BudgetSeconds}
    Resize 900 740 | Out-Null
    try { Guard; [TitleMenuSmoke.Native]::Foreground($window,[uint32]$ownedPid) } catch { Limit ('Normal foreground request denied: '+$_.Exception.GetBaseException().Message) }
    Page 2
    $grip=Control 2230; $style=[LogsInteraction.Native]::Style($grip,[uint32]$ownedPid)
    Assert-True ([TitleMenuSmoke.Native]::Class($grip) -ieq 'Button') '2230 must be native BUTTON'
    Assert-True (($style -band 0x10000) -ne 0 -and ($style -band 0xF) -eq 0xB) '2230 requires WS_TABSTOP + BS_OWNERDRAW'
    Assert-True ((Text 2230) -ceq '拖动调整日志高度（↑↓调整，Home恢复默认）') 'Grip accessible caption differs from agreed contract'
    Assert-Height 240 'first visit default height and dynamic document (not legacy 144/848)'
    Assert-Dark
    $stage='load closed synthetic JSONL through real worker'
    Wait-State 'log file controls initialized' { [TitleMenuSmoke.Native]::IsWindowEnabled((Control 2205)) }
    Click 2205
    Wait-State 'real file listing complete' { (Text 2220).Contains('已刷新') -and [TitleMenuSmoke.Native]::IsWindowEnabled((Control 2206)) }
    $index=[LogsInteraction.Native]::ComboIndex((Control 2209),[uint32]$ownedPid,'ai-relay.1.log')
    Msg (Control 2209) 0x14E ([uint64]$index) | Out-Null
    Click 2206
    Wait-State 'all fixture rows loaded by log worker' { $t=Text 2200; $t.Contains('fixture-row-0000-') -and $t.Contains('fixture-row-0255-') -and (Text 2220).Contains('ai-relay.1.log') -and [TitleMenuSmoke.Native]::IsWindowEnabled((Control 2206)) }
    Click 2202
    Wait-State 'follow paused, stable body' { (Text 2202) -ceq '恢复显示' -and (Text 2220).Contains('已暂停') }
    Assert-True ([regex]::Matches((Text 2200),'"sequence":\d+').Count -eq 256) 'Bounded viewer must contain all 256 fixture lines'
    Msg (Control 2200) 0xB1 8 40 | Out-Null
    $viewerBaseline=[LogsInteraction.Native]::Viewer((Control 2200),[uint32]$ownedPid)
    Assert-True ($viewerBaseline.ReadOnly -and $viewerBaseline.Multiline -and $viewerBaseline.Selection -eq 0x00280008) 'Native viewer baseline/selection invalid'
    Start-Sleep -Milliseconds 250; Assert-Viewer 'paused fixture stability'
    Record 'native viewer baseline; full bounded text read, no injection' $viewerBaseline
    Assert-Files

    $stage='grip keyboard/default/bounds'
    Grip-Key 0x24 240 'Home default'
    Grip-Key 0x28 264 'Down adds 24 DIP'
    Grip-Key 0x26 240 'Up removes 24 DIP'
    Grip-Key 0x26 120 'Up clamps at 120 DIP' 10
    Grip-Key 0x28 720 'Down clamps at 720 DIP' 30
    Grip-Key 0x28 720 'Down at maximum stays clamped'
    Grip-Key 0x24 240 'Home restores default after both bounds'
    $stage='grip live screen-relative drag'
    try {
        $start=Begin-Grip
        $point=Grip-Move $start 24 264 'grip-delta-plus24'
        $point=Grip-Move $start 72 312 'grip-delta-plus72'
        $point=Grip-Move $start -48 192 'grip-delta-minus48'
        $point=Grip-Move $start 120 360 'grip-delta-plus120'
        Release (Control 2230) $point
        Assert-Height 360 'released grip retains chosen height'
    } finally { Cancel-OwnedCapture }
    Page 0; Page 2; Assert-Height 360 'cached page return preserves chosen DIP height'
    Theme 1; Assert-Height 360 'light preview preserves chosen DIP height'
    Theme 2; Assert-Height 360 'dark preview preserves chosen DIP height'; Assert-Dark
    $different=Resize 1000 820
    Assert-Height 360 'larger window does not auto-grow viewer'
    $differentBack=Resize 900 740
    Assert-Height 360 'smaller window preserves chosen DIP height'
    if (-not $different -and -not $differentBack) { Limit 'Work-area clamp prevented distinct outer sizes; resize retention matrix incomplete' }
    $stage='grip Escape rollback'
    try {
        $start=Begin-Grip; $point=Grip-Move $start 48 408 'grip-before-escape'
        Guard; [LogsInteraction.Native]::Key((Control 2230),[uint32]$ownedPid,0x1B)
        Wait-State 'Escape releases grip capture' { (Capture-Owner) -eq [IntPtr]::Zero } 1500
        Assert-Height 360 'Escape restores mouse-down height (not default)'
    } finally { Cancel-OwnedCapture }
    $stage='grip lifecycle capture cleanup'
    foreach ($kind in @('cancel','theme-refresh','resize','page-hide')) {
        try {
            $start=Begin-Grip
            Physical ('grip-'+$kind+'-before-HELD') (Control 2230)
            switch ($kind) {
                'cancel' { Msg (Control 2230) 0x1F | Out-Null }
                'theme-refresh' { Msg $window 0x31A | Out-Null }
                'resize' { if (-not (Resize 960 760)) { Limit 'Grip resize cleanup: work area prevented actual size change' } }
                'page-hide' { Page 0 }
            }
            Wait-State "grip $kind releases capture" { (Capture-Owner) -eq [IntPtr]::Zero } 1500
            if ($kind -eq 'page-hide') { Page 2 }
            Assert-Height 360 ("grip $kind preserves selected height/body/selection"); Assert-Dark
        } finally { Cancel-OwnedCapture }
    }
    Record 'lifecycle scope' 'No-motion held grip cleanup; page-tab clicks can also cause capture loss before hide. Theme-refresh is an owned WM_THEMECHANGED notification, not a system preference change.'
    Grip-Key 0x24 240 'Home restores initial viewer before scrollbar matrix'
    Resize 900 740 | Out-Null

    $stage='dark Edit V/H and viewport V held thumb matrix'
    foreach ($target in @(@(2200,1),@(2200,0),@(2400,1))) { Drag-NC $target[0] $target[1] }
    $stage='real owner timer arrow/page repeat while held'
    foreach ($target in @(@(2200,1),@(2200,0),@(2400,1))) { Repeat-NC $target[0] $target[1] }
    Repeat-NC 2200 1 -PageTrack
    $stage='dark NC capture lifecycle cleanup'
    foreach ($kind in @('escape','cancel','theme-refresh','resize','page-hide')) {
        try {
            $bar=Prepare-NC 2200 1; $h=Control 2200
            $start=[LogsInteraction.Native]::ThumbPoint($bar,1,-1); Begin-NC 2200 1 $start
            $point=[LogsInteraction.Native]::ThumbPoint($bar,1,.25); Mouse $h 0x200 1 $point
            Assert-True ([LogsInteraction.Native]::Scroll($h,[uint32]$ownedPid,1).Position -gt 0) 'NC lifecycle fixture must really move before cancellation'
            Physical ('nc-'+$kind+'-before-HELD') $h 1 ('cleanup-'+$kind)
            switch ($kind) {
                'escape' { Guard; [LogsInteraction.Native]::Key($h,[uint32]$ownedPid,0x1B) }
                'cancel' { Msg $h 0x1F | Out-Null }
                'theme-refresh' { Msg $window 0x31A | Out-Null }
                'resize' { if (-not (Resize 960 760)) { Limit 'NC resize cleanup: work area prevented actual size change' } }
                'page-hide' { Page 0 }
            }
            Wait-State "NC $kind releases capture" { (Capture-Owner) -eq [IntPtr]::Zero } 1500
            if ($kind -eq 'page-hide') { Page 2 }
            Assert-Height 240 ("NC $kind preserves height/body/selection"); Assert-Dark
        } finally { Cancel-OwnedCapture }
    }
    $stage='restore initial log presentation'
    Grip-Key 0x24 240 'final Home restores initial default'
    Msg (Control 2200) 0x115 6 | Out-Null; Msg (Control 2200) 0x114 6 | Out-Null; Scroll 6
    Assert-Height 240 'final default geometry'; Assert-Dark; Assert-Files
    $defaultRestored=$true
    Assert-True ([TitleMenuSmoke.Native]::IsWindowEnabled((Control 2102)) -and -not [TitleMenuSmoke.Native]::IsWindowEnabled((Control 2103))) 'Proxy must remain stopped throughout'
    Physical 'final-dark-default-REFERENCE-NOT-held-acceptance'
    Record 'scope' 'No direct WM_APP+90 height command, no manufactured scroll ranges, no WM_TIMER injection, no forced repaint/PrintWindow for acceptance. Height changes are driven by owned native grip keys/mouse; JSONL is loaded through the real read worker.'
    $matrixFinished=$true
} catch {
    $reason=$_.Exception.GetBaseException().Message
    if ($reason.StartsWith('SKIP fixture:')) { Limit $reason } else { $failure=$reason; Record ('FAIL at '+$stage) $reason }
} finally {
    if ($null -ne $child -and -not $child.HasExited) {
        try {
            Wait-Identity -Cleanup
            if ($window -eq [IntPtr]::Zero) { $window=[TitleMenuSmoke.Native]::FindMain([uint32]$ownedPid) }
            if ($window -ne [IntPtr]::Zero) {
                Cancel-OwnedCapture
                # Restore only through the genuine grip Home path, never the private
                # height notification. Missing controls/early startup are reported.
                $restore=[TitleMenuSmoke.Native]::FindControl($window,[uint32]$ownedPid,2230)
                if ($actualDpi -eq 96 -and $restore -ne [IntPtr]::Zero -and [TitleMenuSmoke.Native]::IsWindowVisible($restore)) {
                    [LogsInteraction.Native]::Tree($window,$restore,[uint32]$ownedPid)
                    [LogsInteraction.Native]::Key($restore,[uint32]$ownedPid,0x24)
                    $defaultRestored=([TitleMenuSmoke.Native]::Outer(([TitleMenuSmoke.Native]::FindControl($window,[uint32]$ownedPid,2200)),[uint32]$ownedPid).Height -eq 240)
                }
            }
        } catch { Limit ('Owned capture/Home cleanup: '+$_.Exception.GetBaseException().Message) }
        try {
            Wait-Identity -Cleanup
            if ($window -ne [IntPtr]::Zero) {
                [TitleMenuSmoke.Native]::Post($window,[uint32]$ownedPid,0x111,1002,0)
                $remaining=[Math]::Max(0,[Math]::Min(7000,[int](($BudgetSeconds-$clock.Elapsed.TotalSeconds)*1000)))
                $normalExit=$child.WaitForExit($remaining)
                if ($normalExit) { Assert-True ($child.ExitCode -eq 0) 'Owned exit command returned nonzero exit code' }
            }
            Record 'owned command 1002 exit; no kill fallback' $normalExit
        } catch {
            $exitError=$_.Exception.GetBaseException().Message
            if (-not $failure) { $failure=$exitError }
            Record 'Owned exit failure (no kill fallback)' $exitError
        }
    }
    if ($null -ne $child -and -not $normalExit -and -not $failure) { $failure='Owned normal exit not confirmed; no kill attempted' }
    if ($null -ne $configHash) {
        try { Assert-Files; Record 'config and closed JSONL SHA256 unchanged after owned-exit attempt' @{Unchanged=$true;NormalExitConfirmed=$normalExit} }
        catch { if (-not $failure) { $failure=$_.Exception.GetBaseException().Message }; Record 'file integrity failure' $_.Exception.GetBaseException().Message }
    }
    if ($null -ne $child -and -not $defaultRestored) { Limit 'Initial viewer height 240 DIP was not confirmed restored before exit' }
    try { No-Instance; Record 'post-exit process/mutex absent' $true }
    catch { if (-not $failure) { $failure=$_.Exception.GetBaseException().Message }; Record 'post-exit guard failed (no foreign action)' $_.Exception.GetBaseException().Message }
    if ($oldDpi -ne [IntPtr]::Zero) { [void][TitleMenuSmoke.Native]::SetThreadDpiAwarenessContext($oldDpi) }
    $seconds=if ($null -eq $clock) { 0 } else { [Math]::Round($clock.Elapsed.TotalSeconds,3) }
    $report=[pscustomobject]@{
        Passed=($null -eq $failure -and $normalExit -and $matrixFinished); Complete=($null -eq $failure -and $normalExit -and $matrixFinished -and $notRun.Count -eq 0)
        Failure=$failure;Stage=$stage;Exe=$exe;PID=$ownedPid;Seconds=$seconds;BudgetSeconds=$BudgetSeconds;MatrixFinished=$matrixFinished
        NormalExit=$normalExit;DefaultHeightRestored=$defaultRestored;ActualDPI=$actualDpi;ActualDpiScope='96 only; other DPI/high contrast cause explicit incomplete fixture skip'
        ConfigSHA256=$configHash;ClosedJSONLSHA256=$fixtureHash;TempRetained=$temp
        Checks=$events.ToArray();Frames=$frames.ToArray();HeldPixelFrames=$heldPixels;NotRun=$notRun.ToArray()
        Limits=@(
            'HWND-local scalar mouse/key messages, NOT physical hardware/SendInput/real pointer/button-state or physical Tab acceptance. Capture, native pos and native thumb are independently checked while no up has been sent.'
            'Physical captures are bounded samples after moves and during holds, not continuous video or proof against every sub-frame flash. Occlusion/offscreen skips never use PrintWindow as replacement and keep Complete=false.'
            'Only native Edit2200 V/H and viewport2400 V tracking/repeat are exercised; ListView report driver, other edit controls and light native modal scrollbar interactions are NOT exercised.'
            'Keyboard Home/up/down/Escape are directed to owned grip; not an IsDialogMessage/physical keyboard or screen-reader acceptance. Selection/read-only style and full bounded body hash are preserved, clipboard is untouched.'
            'Page-tab transitions can release previous capture before hide; this checks transition cleanup, not an isolated WM_SHOWWINDOW cause. WM_THEMECHANGED reapplication is tested without changing OS theme.'
            'Resize requests use SWP_ASYNCWINDOWPOS and bounded full-rectangle plus active-log descendant layout observation. This assumes separate input queues (the harness never attaches them); it is not an arbitrary hostile/attached-queue timeout guarantee. Startup waits only for empty managed Path metadata while raw handle/PID/start time/actual EXE remain strict.'
            'Session-only height retention, not cross-restart persistence; config/closed fixture file hashes are checked. App-created history/identity files in the private temp are permitted and retained, not claimed byte-unchanged.'
            'No Cargo/build, real upstream/mock sockets, listener start, config Save/Reload, file deletion, Explorer, Startup/firewall, clipboard, global input, foreign IPC or kill fallback. Old scripts contribute declarations only.'
        )
    }
    $report | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath (Join-Path $output 'report.json') -Encoding UTF8
    if ($null -ne $child) { $child.Dispose() }
    Write-Output ('REPORT '+(Join-Path $output 'report.json'))
}
if ($failure) { throw $failure }
if ($notRun.Count) { Write-Output 'INCOMPLETE: inspect NotRun and physical evidence; no full visual acceptance claimed.' }
else { Write-Output 'PASS: bounded actual-96-DPI grip/held-scrollbar matrix; see explicit limitations in report.json.' }
