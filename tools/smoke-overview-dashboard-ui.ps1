# Windows PowerShell 5.1 x64 / interactive Windows 10/11 desktop.
# Only our pinned child, private config and loopback mock. Never builds or runs old tests.
# PrintWindow OFFSCREEN renders only: NOT desktop, occlusion, OCR or physical-input acceptance.
# Run -ValidateOnly first; the primary agent exclusively owns any later GUI/Cargo run.
[CmdletBinding()]
param(
    [string]$ExePath,
    [switch]$ValidateOnly
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([IntPtr]::Size -ne 8) { throw 'Use a fresh 64-bit powershell.exe -NoProfile process' }
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $projectRoot 'target\debug\ai-relay.exe'
} elseif (-not [IO.Path]::IsPathRooted($ExePath)) {
    $ExePath = Join-Path $projectRoot $ExePath
}
$ExePath = [IO.Path]::GetFullPath($ExePath)

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
using System.Drawing.Imaging;
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
    [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h,ref StatusBarNative.Point point);
    [DllImport("gdi32.dll")] static extern IntPtr CreateCompatibleDC(IntPtr dc);
    [DllImport("gdi32.dll")] static extern bool DeleteDC(IntPtr dc);
    [DllImport("gdi32.dll",CharSet=CharSet.Unicode,ExactSpelling=true)] static extern IntPtr CreateFontW(int height,int width,int escapement,int orientation,int weight,uint italic,uint underline,uint strikeout,uint charset,uint outputPrecision,uint clipPrecision,uint quality,uint pitchAndFamily,string face);
    [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr obj);
    [DllImport("gdi32.dll")] static extern IntPtr SelectObject(IntPtr dc,IntPtr obj);
    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
    [DllImport("user32.dll")] static extern uint GetGuiResources(IntPtr process,uint flags);
    [DllImport("gdi32.dll",CharSet=CharSet.Unicode)] static extern bool GetTextExtentPoint32W(IntPtr dc,string text,int length,out Size size);
    [StructLayout(LayoutKind.Sequential)] struct Size { public int Width,Height; }
    public static IntPtr[] Descendants(IntPtr root,uint pid) {
        ProxyUiSmoke.CheckOwner(root,pid);
        var found=new List<IntPtr>(); var pending=new Queue<IntPtr>(); var seen=new HashSet<IntPtr>();
        pending.Enqueue(root); seen.Add(root);
        while(pending.Count!=0) foreach(var h in ModernNative.DirectChildren(pending.Dequeue(),pid)) {
            // Startup reparents page controls into 2400 while this read-only
            // walk is in flight; an owned HWND can appear in both snapshots.
            if(!seen.Add(h)) continue;
            if(found.Count>=4096) throw new Exception("Recursive HWND enumeration exceeded bound");
            found.Add(h); pending.Enqueue(h);
        }
        return found.ToArray();
    }
    public static IntPtr FindControl(IntPtr root,int id,uint pid) {
        IntPtr found=IntPtr.Zero;
        foreach(var h in Descendants(root,pid)) if(ClarityNative.GetDlgCtrlID(h)==id) {
            if(found!=IntPtr.Zero) throw new Exception("Ambiguous recursive control ID: "+id);
            found=h;
        }
        // Missing children are normal during startup; ownership failures are not.
        if(found!=IntPtr.Zero) ProxyUiSmoke.CheckOwner(found,pid);
        return found;
    }
    public static StatusBarNative.Rect ClientOnScreen(IntPtr h,uint pid) {
        ProxyUiSmoke.CheckOwner(h,pid); var previous=StatusBarNative.EnterDpi();
        try {
            var r=StatusBarNative.Client(h,pid); var p=new StatusBarNative.Point();
            if(!ClientToScreen(h,ref p)) throw new Exception("Absolute client origin unavailable");
            return new StatusBarNative.Rect { Left=p.X,Top=p.Y,Right=p.X+r.Width,Bottom=p.Y+r.Height };
        } finally { StatusBarNative.RestoreDpi(previous); }
    }
    public const string ExpectedFontWidthScope="Harness-owned memory DC/CreateFontW reproduces src/windows/mod.rs Font::new: height=-scale(14,dpi), weight=400, charset=1, quality=5, face=Microsoft YaHei UI, other parameters=0. Expected painter-font contract budgets only; NOT target HFONT measurement, font-identity verification, internal painter rectangles or actual-fit proof.";
    sealed class ExpectedContractFontDc : IDisposable {
        public IntPtr Dc { get; private set; }
        IntPtr font,previous;
        public ExpectedContractFontDc(uint dpi) {
            if(dpi==0) throw new ArgumentException("Expected font contract requires actual nonzero DPI");
            Dc=CreateCompatibleDC(IntPtr.Zero);
            if(Dc==IntPtr.Zero) throw new InvalidOperationException("Cannot create harness-owned font measurement memory DC");
            try {
                // This process owns BOTH objects. A remote WM_GETFONT result is
                // opaque and must NEVER be selected into a harness DC.
                font=CreateFontW(-ModernNative.Scale(14,dpi),0,0,0,400,0,0,0,1,0,0,5,0,"Microsoft YaHei UI");
                if(font==IntPtr.Zero) throw new InvalidOperationException("Cannot create harness expected painter-font contract");
                previous=SelectObject(Dc,font);
                if(previous==IntPtr.Zero || previous==new IntPtr(-1)) {
                    previous=IntPtr.Zero;
                    throw new InvalidOperationException("Cannot select harness-owned expected-contract font");
                }
            } catch { Dispose(); throw; }
        }
        public void Dispose() {
            bool cleanup=true;
            if(Dc!=IntPtr.Zero) {
                if(previous!=IntPtr.Zero) {
                    var restored=SelectObject(Dc,previous);
                    if(restored==IntPtr.Zero || restored==new IntPtr(-1)) cleanup=false;
                    previous=IntPtr.Zero;
                }
                // Destroy the private DC before its font, also detaching the font
                // if restoring the original selection unexpectedly failed.
                if(!DeleteDC(Dc)) cleanup=false;
                Dc=IntPtr.Zero;
            }
            if(font!=IntPtr.Zero) { if(!DeleteObject(font)) cleanup=false; font=IntPtr.Zero; }
            if(!cleanup) throw new InvalidOperationException("Harness expected-font/DC restore or deletion failed");
        }
    }
    static int[] ExpectedContractTextWidths(uint dpi,string[] text) {
        using(var owned=new ExpectedContractFontDc(dpi)) {
            if(text==null || text.Length<1 || text.Length>16) throw new ArgumentException("Expected-contract text batch exceeds bound");
            var widths=new int[text.Length];
            for(int i=0;i<text.Length;i++) {
                Size size;
                if(String.IsNullOrEmpty(text[i]) || text[i].Length>512) throw new ArgumentException("Expected-contract text length invalid");
                if(!GetTextExtentPoint32W(owned.Dc,text[i],text[i].Length,out size) || size.Width<=0)
                    throw new InvalidOperationException("Harness expected painter-font contract measurement failed");
                widths[i]=size.Width;
            }
            return widths;
        }
    }
    public static string ValidateExpectedFont() {
        var text=new[]{"请求 0","错误 未知","上传 100.00","下载 0","区间均值 · 次/s","区间均值 · B/s"};
        var dpis=new uint[]{96,120,144,192}; var warm=new List<int[]>();
        foreach(uint dpi in dpis) warm.Add(ExpectedContractTextWidths(dpi,text));
        uint before=GetGuiResources(GetCurrentProcess(),0);
        for(int round=0;round<4;round++) for(int i=0;i<dpis.Length;i++) {
            var widths=ExpectedContractTextWidths(dpis[i],text);
            for(int j=0;j<widths.Length;j++) if(widths[j]!=warm[i][j]) throw new Exception("Harness expected-font contract width was unstable");
        }
        int rejected=0;
        foreach(uint dpi in dpis) {
            try { ExpectedContractTextWidths(dpi,new[]{String.Empty}); } catch(ArgumentException) { rejected++; }
        }
        uint after=GetGuiResources(GetCurrentProcess(),0);
        if(rejected!=dpis.Length || after!=before) throw new Exception("Harness expected-font/DC success/error cleanup self-check failed: GDI "+before+" -> "+after);
        return "harness-owned expected-font/memory-DC self-checks at 96/120/144/192 DPI: 16 repeated measurements and four invalid-text cleanup paths; own-process GDI "+before+" -> "+after+"; NOT target-font, HWND, painter-fit or visual acceptance";
    }
    public sealed class CaptureAnalysis {
        public string Path,Scope="Read-only saved PNG analysis; NOT a new capture, HWND/font test or acceptance pass";
        public int Width,Height,Colors,Samples,Bright,Untouched,Tiles,BrightTiles,FirstUntouchedY=-1,LastUntouchedY=-1;
        public double MeanLuminance,BrightFraction,BrightTileFraction,UntouchedFraction;
        public bool BlankOrIncomplete;
    }
    public static CaptureAnalysis AnalyzePng(string path) {
        path=System.IO.Path.GetFullPath(path);
        using(var bitmap=new Bitmap(path)) return AnalyzeBitmap(bitmap,path);
    }
    static CaptureAnalysis AnalyzeBitmap(Bitmap bitmap,string path) {
        var proof=new CaptureAnalysis { Path=path };
        {
            proof.Width=bitmap.Width; proof.Height=bitmap.Height;
            if(proof.Width<1 || proof.Height<1 || proof.Width>8192 || proof.Height>8192) throw new Exception("Saved capture exceeds bounded PNG dimensions");
            var colors=new HashSet<int>(); double sum=0;
            // Match ModernNative.Capture's full-PNG sample/tile loops and guard,
            // including magenta initialization; do not crop or relax its threshold.
            for(int y=0;y<bitmap.Height;y+=2) for(int x=0;x<bitmap.Width;x+=2) {
                var c=bitmap.GetPixel(x,y); colors.Add(c.ToArgb()); proof.Samples++;
                sum+=0.2126*c.R+0.7152*c.G+0.0722*c.B;
                if(c.R>=220 && c.G>=220 && c.B>=220) proof.Bright++;
                if(c.R==255 && c.G==0 && c.B==255) {
                    proof.Untouched++;
                    if(proof.FirstUntouchedY<0) proof.FirstUntouchedY=y;
                    proof.LastUntouchedY=y;
                }
            }
            for(int y=0;y+12<=bitmap.Height;y+=12) for(int x=0;x+12<=bitmap.Width;x+=12) {
                int bright=0,count=0;
                for(int dy=0;dy<12;dy+=2) for(int dx=0;dx<12;dx+=2) {
                    var c=bitmap.GetPixel(x+dx,y+dy); count++;
                    if(c.R>=220 && c.G>=220 && c.B>=220) bright++;
                }
                proof.Tiles++; if(bright>=count*0.8) proof.BrightTiles++;
            }
            proof.Colors=colors.Count; proof.MeanLuminance=sum/proof.Samples;
            proof.BrightFraction=(double)proof.Bright/proof.Samples;
            proof.BrightTileFraction=proof.Tiles==0?0:(double)proof.BrightTiles/proof.Tiles;
            proof.UntouchedFraction=(double)proof.Untouched/proof.Samples;
            proof.BlankOrIncomplete=proof.Untouched>proof.Samples/100 || proof.Colors<2;
        }
        return proof;
    }
    public sealed class CropGeometry {
        public StatusBarNative.Rect MainOuterScreen,HostClientScreen,DashboardScreen,DashboardClientScreen,CropInWhole;
        public uint Dpi;
        public string CaptureKind="wholewindowoffscreen-completeviewport-containedcrop";
    }
    static bool SameRect(StatusBarNative.Rect a,StatusBarNative.Rect b) {
        return a.Left==b.Left && a.Top==b.Top && a.Right==b.Right && a.Bottom==b.Bottom;
    }
    static bool Contains(StatusBarNative.Rect outer,StatusBarNative.Rect inner) {
        return inner.Width>0 && inner.Height>0 && inner.Left>=outer.Left && inner.Top>=outer.Top && inner.Right<=outer.Right && inner.Bottom<=outer.Bottom;
    }
    static StatusBarNative.Rect CompleteCropRect(StatusBarNative.Rect whole,StatusBarNative.Rect host,StatusBarNative.Rect dashboard) {
        if(whole.Width<1 || whole.Height<1 || whole.Width>8192 || whole.Height>8192 || !Contains(whole,host) || !Contains(host,dashboard) || !Contains(whole,dashboard))
            throw new InvalidOperationException("FULL native dashboard rectangle must be contained by the host CLIENT clip and whole-window bounds; partial intersections are forbidden");
        return new StatusBarNative.Rect { Left=dashboard.Left-whole.Left,Top=dashboard.Top-whole.Top,
            Right=dashboard.Right-whole.Left,Bottom=dashboard.Bottom-whole.Top };
    }
    public static CropGeometry DashboardCropBounds(IntPtr main,IntPtr host,IntPtr dashboard,uint pid) {
        ProxyUiSmoke.CheckOwner(main,pid); ProxyUiSmoke.CheckOwner(host,pid); ProxyUiSmoke.CheckOwner(dashboard,pid);
        var previous=StatusBarNative.EnterDpi();
        try {
            if(StatusBarNative.GetParent(host)!=main || StatusBarNative.GetParent(dashboard)!=host || ClarityNative.GetDlgCtrlID(dashboard)!=2109 || !ProxyUiSmoke.IsWindowVisible(dashboard))
                throw new InvalidOperationException("Expected visible pinned dashboard 2109 under the actual main/viewport tree");
            var proof=new CropGeometry { MainOuterScreen=StatusBarNative.Outer(main,pid),HostClientScreen=ClientOnScreen(host,pid),
                DashboardScreen=StatusBarNative.Outer(dashboard,pid),DashboardClientScreen=ClientOnScreen(dashboard,pid),Dpi=ClarityNative.GetDpiForWindow(dashboard) };
            if(proof.Dpi==0 || !SameRect(proof.DashboardScreen,proof.DashboardClientScreen))
                throw new InvalidOperationException("Borderless owner-draw dashboard full HWND/client coordinates must agree; never shrink the crop to hide missing native area");
            proof.CropInWhole=CompleteCropRect(proof.MainOuterScreen,proof.HostClientScreen,proof.DashboardScreen);
            return proof;
        } finally { StatusBarNative.RestoreDpi(previous); }
    }
    static void RequireSameCrop(CropGeometry before,CropGeometry after) {
        if(before==null || before.Dpi!=after.Dpi || !SameRect(before.MainOuterScreen,after.MainOuterScreen) ||
            !SameRect(before.HostClientScreen,after.HostClientScreen) || !SameRect(before.DashboardScreen,after.DashboardScreen) ||
            !SameRect(before.DashboardClientScreen,after.DashboardClientScreen) || !SameRect(before.CropInWhole,after.CropInWhole))
            throw new InvalidOperationException("Native screen/client geometry changed across whole-window PrintWindow/crop; no stable full-dashboard proof");
    }
    static Bitmap CopyCompleteDashboard(Bitmap source,CropGeometry proof) {
        var rect=CompleteCropRect(proof.MainOuterScreen,proof.HostClientScreen,proof.DashboardScreen);
        if(source.Width!=proof.MainOuterScreen.Width || source.Height!=proof.MainOuterScreen.Height || !SameRect(rect,proof.CropInWhole))
            throw new InvalidOperationException("Whole PNG dimensions/origin do not match measured native geometry (including title/non-client frame)");
        // Exact 1:1 copy of the COMPLETE rectangle; no Intersect, resize or stitching.
        return source.Clone(new Rectangle(rect.Left,rect.Top,rect.Width,rect.Height),PixelFormat.Format32bppArgb);
    }
    public static ModernNative.Shot CropDashboard(string wholePath,string cropPath,CropGeometry proof,IntPtr main,IntPtr host,IntPtr dashboard,uint pid) {
        ProxyUiSmoke.CheckOwner(main,pid); ProxyUiSmoke.CheckOwner(host,pid); ProxyUiSmoke.CheckOwner(dashboard,pid);
        var work=Task.Run(delegate {
            RequireSameCrop(proof,DashboardCropBounds(main,host,dashboard,pid));
            using(var source=new Bitmap(wholePath)) using(var crop=CopyCompleteDashboard(source,proof)) {
                crop.Save(cropPath,ImageFormat.Png); // Preserve failures; a saved PNG is not automatically accepted.
                var analysis=AnalyzeBitmap(crop,cropPath);
                if(analysis.BlankOrIncomplete) throw new InvalidOperationException("Blank/incomplete wholewindowoffscreen-completeviewport-containedcrop: "+cropPath);
                RequireSameCrop(proof,DashboardCropBounds(main,host,dashboard,pid));
                return new ModernNative.Shot { Path=cropPath,CaptureKind="wholewindowoffscreen-completeviewport-containedcrop (PrintWindow OFFSCREEN, NOT desktop capture)",
                    Width=analysis.Width,Height=analysis.Height,Colors=analysis.Colors,MeanLuminance=analysis.MeanLuminance,
                    BrightFraction=analysis.BrightFraction,BrightTileFraction=analysis.BrightTileFraction };
            }
        });
        if(!work.Wait(8000)) throw new TimeoutException("Complete dashboard crop/analysis exceeded eight seconds: "+cropPath);
        return work.GetAwaiter().GetResult();
    }
    public static string ValidateCrop() {
        // Pure helper tests only. Synthetic coordinates/raster are NOT application
        // geometry, fonts, plots, screenshots or visual acceptance evidence.
        var whole=new StatusBarNative.Rect { Left=-1800,Top=90,Right=-1000,Bottom=770 };
        var host=new StatusBarNative.Rect { Left=-1792,Top=165,Right=-1025,Bottom=722 };
        var dash=new StatusBarNative.Rect { Left=-1768,Top=215,Right=-1049,Bottom=697 };
        var rect=CompleteCropRect(whole,host,dash);
        if(rect.Left!=32 || rect.Top!=125 || rect.Width!=719 || rect.Height!=482) throw new Exception("Negative-monitor / non-client crop offset self-check failed");
        int rejected=0;
        foreach(var bad in new[]{
            new StatusBarNative.Rect { Left=dash.Left,Top=host.Top-1,Right=dash.Right,Bottom=host.Top+481 },
            new StatusBarNative.Rect { Left=dash.Left,Top=host.Bottom-481,Right=dash.Right,Bottom=host.Bottom+1 },
            new StatusBarNative.Rect { Left=host.Left-1,Top=dash.Top,Right=dash.Right,Bottom=dash.Bottom },
            new StatusBarNative.Rect { Left=dash.Left,Top=dash.Top,Right=host.Right+1,Bottom=dash.Bottom },
            new StatusBarNative.Rect { Left=dash.Left,Top=dash.Top,Right=dash.Left,Bottom=dash.Bottom } }) {
            try { CompleteCropRect(whole,host,bad); } catch(InvalidOperationException) { rejected++; }
        }
        var minimumHost=host; minimumHost.Bottom=minimumHost.Top+437;
        try { CompleteCropRect(whole,minimumHost,dash); } catch(InvalidOperationException) { rejected++; }
        if(rejected!=6) throw new Exception("A partial/zero-area/too-tall crop was accepted");
        int invalidGeometry=0;
        var outsideWhole=host; outsideWhole.Left=whole.Left-1;
        try { CompleteCropRect(whole,outsideWhole,dash); } catch(InvalidOperationException) { invalidGeometry++; }
        var smallWhole=new StatusBarNative.Rect { Left=100,Top=-200,Right=130,Bottom=-172 };
        var smallHost=new StatusBarNative.Rect { Left=102,Top=-196,Right=128,Bottom=-174 };
        var smallDash=new StatusBarNative.Rect { Left=104,Top=-193,Right=123,Bottom=-177 };
        var proof=new CropGeometry { MainOuterScreen=smallWhole,HostClientScreen=smallHost,DashboardScreen=smallDash,
            DashboardClientScreen=smallDash,CropInWhole=CompleteCropRect(smallWhole,smallHost,smallDash),Dpi=96 };
        using(var source=new Bitmap(30,28)) {
            for(int y=0;y<28;y++) for(int x=0;x<30;x++) source.SetPixel(x,y,Color.FromArgb(255,x*7,y*8,(x+y)*4));
            using(var crop=CopyCompleteDashboard(source,proof)) {
                if(crop.Width!=19 || crop.Height!=16) throw new Exception("Exact full-raster crop dimensions failed");
                for(int y=0;y<crop.Height;y++) for(int x=0;x<crop.Width;x++)
                    if(crop.GetPixel(x,y)!=source.GetPixel(x+4,y+7)) throw new Exception("1:1 full-raster crop offset/content mismatch");
                if(AnalyzeBitmap(crop,null).BlankOrIncomplete) throw new Exception("Nonblank test raster rejected");
            }
            var shortened=new CropGeometry { MainOuterScreen=smallWhole,HostClientScreen=smallHost,DashboardScreen=smallDash,
                DashboardClientScreen=smallDash,CropInWhole=proof.CropInWhole,Dpi=96 };
            shortened.CropInWhole.Bottom--;
            try { using(var invalid=CopyCompleteDashboard(source,shortened)) { } } catch(InvalidOperationException) { invalidGeometry++; }
            using(var wrongSize=new Bitmap(29,28)) {
                try { using(var invalid=CopyCompleteDashboard(wrongSize,proof)) { } } catch(InvalidOperationException) { invalidGeometry++; }
            }
            try { RequireSameCrop(proof,shortened); } catch(InvalidOperationException) { invalidGeometry++; }
            using(var graphics=Graphics.FromImage(source)) graphics.Clear(Color.Magenta);
            if(!AnalyzeBitmap(source,null).BlankOrIncomplete) throw new Exception("Unpainted magenta raster accepted");
            using(var graphics=Graphics.FromImage(source)) graphics.Clear(Color.White);
            if(!AnalyzeBitmap(source,null).BlankOrIncomplete) throw new Exception("Single-color raster accepted");
        }
        if(invalidGeometry!=4) throw new Exception("Whole bounds, shortened crop, PNG dimensions or geometry change was not rejected");
        return "pure crop geometry/raster self-checks: negative-screen/non-client offset, exact 1:1 copy, six partial/too-tall and four inconsistent-geometry rejections, unchanged magenta/single-color guard; no native GUI/visual acceptance";
    }
    public static string ExceptionDetails(Exception error) {
        var lines=new List<string>();
        for(int depth=0;error!=null && depth<16;depth++,error=error.InnerException) {
            var aggregate=error as AggregateException;
            if(aggregate!=null) {
                foreach(var inner in aggregate.Flatten().InnerExceptions) lines.Add(inner.ToString());
                break;
            }
            lines.Add(error.GetType().FullName+": "+error.Message);
            if(error.InnerException==null && error.StackTrace!=null) lines.Add(error.StackTrace);
        }
        return String.Join(Environment.NewLine,lines.ToArray());
    }
    public sealed class RegionProof {
        public string Name,Role,WidthScope;
        public int X,Y,Width,Height,Blue,Red,Ink,Colored,NeutralInk,NeutralSurface,ExpectedContractTextWidth,ExpectedLayoutTextBudget;
    }
    // Actual full HWND/PNG/DPI, but font widths are HARNESS-OWNED expected contract
    // budgets, never target-HFONT measurements or exported painter rectangles.
    // Pixel presence is NOT OCR, glyph identity or actual-fit proof.
    public static RegionProof[] Inspect(string path,IntPtr dashboard,uint pid,bool dark,string[] labels,bool unknownReason) {
        ProxyUiSmoke.CheckOwner(dashboard,pid);
        uint dpi=ClarityNative.GetDpiForWindow(dashboard);
        if(dpi==0 || labels==null || labels.Length!=4) throw new Exception("Missing actual DPI/metadata labels");
        var client=StatusBarNative.Client(dashboard,pid);
        if(client.Width<ModernNative.Scale(696,dpi) || client.Height<ModernNative.Scale(350,dpi))
            throw new Exception("Dashboard is in its real text fallback; no mock plot or curve acceptance allowed");
        // Read-only assignment check only: do not select/read/delete or replace
        // the target process's HFONT, and do not claim its identity was verified.
        if(ModernNative.Message(dashboard,pid,0x31,0,0)==0) throw new InvalidOperationException("Target dashboard WM_GETFONT returned zero");
        var contractWidths=ExpectedContractTextWidths(dpi,new[]{labels[0],labels[1],labels[2],labels[3],"区间均值 · 次/s","区间均值 · B/s"});
        Func<int,int> px=delegate(int dip) { return ModernNative.Scale(dip,dpi); };
        var result=new List<RegionProof>();
        using(var image=new Bitmap(path)) {
            if(image.Width!=client.Width || image.Height!=client.Height) throw new Exception("PNG dimensions must equal the complete native dashboard, cropped only after full viewport containment");
            int gap=px(12),cardWidth=(client.Width-3*gap)/4;
            for(int i=0;i<4;i++) {
                int x=i*(cardWidth+gap),w=i==3?client.Width-x:cardWidth;
                result.Add(Region(image,"card-"+i+"-badge","badge",x+px(12),px(12),px(22),px(22),dark));
                result.Add(Region(image,"card-"+i+"-value","value",x+px(12),px(40),w-px(24),px(36),dark));
            }
            int panelWidth=(client.Width-gap)/2,top=px(124),bottom=client.Height-px(24);
            for(int i=0;i<2;i++) {
                int panelLeft=i*(panelWidth+gap),panelRight=i==0?panelWidth:client.Width;
                int left=panelLeft+px(14),right=panelRight-px(14),unitLeft=Math.Max(left,right-contractWidths[4+i]);
                result.Add(Region(image,"chart-"+i+"-title","title",left,top+px(8),unitLeft-gap-left,px(30)-px(8),dark));
                var unit=Region(image,"chart-"+i+"-units","units",unitLeft,top+px(8),right-unitLeft,px(30)-px(8),dark);
                unit.ExpectedContractTextWidth=contractWidths[4+i]; unit.ExpectedLayoutTextBudget=unit.Width; unit.WidthScope=ExpectedFontWidthScope; result.Add(unit);
                // Expected compact items use CONTRACT extents + 22 DIP (14 glyph + 8 gap),
                // then a 12-DIP item gap; never divide normal labels into panel halves.
                int available=Math.Max(0,right-left-gap),marker=px(22);
                int a=contractWidths[2*i]+marker,b=contractWidths[2*i+1]+marker;
                int first=a+b<=available?a:Math.Min(a,available/2);
                int second=Math.Min(b,Math.Max(0,available-first));
                for(int j=0;j<2;j++) {
                    int x=j==0?left:left+first+gap,w=j==0?first:second;
                    string role=i==0?(j==0?"legend-blue":"legend-red"):(j==0?"legend-neutral":"legend-blue");
                    result.Add(Region(image,"chart-"+i+"-legend-"+j+"-glyph",role,x,top+px(34),px(14),px(20),dark));
                    var label=Region(image,"chart-"+i+"-legend-"+j+"-text","legend-text",x+marker,top+px(34),w-marker,px(20),dark);
                    label.ExpectedContractTextWidth=contractWidths[2*i+j]; label.ExpectedLayoutTextBudget=label.Width; label.WidthScope=ExpectedFontWidthScope; result.Add(label);
                }
                if(unknownReason) result.Add(Region(image,"chart-"+i+"-unknown-reason","unknown-reason",panelLeft+px(62),top+px(58),right-panelLeft-px(62),px(18),dark));
                int plotTop=top+px(unknownReason?88:68),plotBottom=bottom-px(32);
                result.Add(Region(image,"chart-"+i+"-plot","plot",panelLeft+px(62),plotTop,panelRight-px(18)-panelLeft-px(62),plotBottom-plotTop,dark));
            }
        }
        return result.ToArray();
    }
    public static void RequireBadgeInk(RegionProof proof,bool error) {
        if(proof.Ink<4 || proof.NeutralSurface<proof.Width*proof.Height/2)
            throw new Exception("Missing readable badge icon or neutral surface: "+proof.Name);
        if(error?proof.Red<4:(proof.NeutralInk<4 || proof.Colored!=0))
            throw new Exception("Badge must use neutral ink except for a nonzero error count: "+proof.Name);
    }
    public static string ValidatePresentation() {
        foreach(bool dark in new[]{false,true}) using(var image=new Bitmap(22,22)) {
            Color surface=dark?Color.FromArgb(51,51,51):Color.FromArgb(245,245,245);
            Color ink=dark?Color.FromArgb(197,197,197):Color.FromArgb(97,97,97);
            using(var graphics=Graphics.FromImage(image)) graphics.Clear(surface);
            int rejected=0;
            try { RequireBadgeInk(Region(image,"blank","badge",0,0,22,22,dark),false); }
            catch(Exception) { rejected++; }
            for(int x=5;x<17;x++) image.SetPixel(x,10,ink);
            RequireBadgeInk(Region(image,"neutral","badge",0,0,22,22,dark),false);
            for(int x=5;x<17;x++) image.SetPixel(x,10,dark?Color.FromArgb(228,154,139):Color.FromArgb(179,80,63));
            try { RequireBadgeInk(Region(image,"wrong-color","badge",0,0,22,22,dark),false); }
            catch(Exception) { rejected++; }
            RequireBadgeInk(Region(image,"error","badge",0,0,22,22,dark),true);
            if(rejected!=2) throw new Exception("Blank or unexpected colored badge was accepted");
        }
        return "neutral badge surface/ink and nonzero-error red checks, blank and unexpected-colored icon rejection in both themes; synthetic helper only";
    }
    static RegionProof Region(Bitmap b,string name,string role,int x,int y,int w,int h,bool dark) {
        if(w<1 || h<1 || x<0 || y<0 || x+w>b.Width || y+h>b.Height) throw new Exception("Expected inspection region outside complete viewport-contained dashboard crop: "+name);
        var p=new RegionProof { Name=name,Role=role,X=x,Y=y,Width=w,Height=h };
        // Inspect every pixel so 1px legend dashes are not missed at fractional DPI.
        for(int yy=y;yy<y+h;yy++) for(int xx=x;xx<x+w;xx++) {
            var c=b.GetPixel(xx,yy);
            if(c.B>c.R+25 && c.B>c.G+12) p.Blue++;
            if(c.R>c.G+25 && c.R>c.B+25) p.Red++;
            int span=Math.Max(c.R,Math.Max(c.G,c.B))-Math.Min(c.R,Math.Min(c.G,c.B));
            if(span>45) p.Colored++;
            int l=(int)(0.2126*c.R+0.7152*c.G+0.0722*c.B);
            bool ink=dark?l>150:l<125;
            if(ink) p.Ink++;
            if(span<=8 && ink) p.NeutralInk++;
            if(span<=8 && (dark?l>=30 && l<=100:l>=180)) p.NeutralSurface++;
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
$tokens=$null; $parseErrors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($PSCommandPath,[ref]$tokens,[ref]$parseErrors)
if ($parseErrors.Count -gt 0) { throw 'PowerShell parser errors' }
foreach ($call in $ast.FindAll({ param($n) $n -is [Management.Automation.Language.InvokeMemberExpressionAst] -and $n.Static },$true)) {
    if ($call.Expression.Extent.Text -match '^\[(ProxyUiSmoke|ClarityNative|StatusBarNative|ModernNative|WindowSmoke|DashboardNative|DashboardMock)\]$') {
        $helperType=$Matches[1] -as [type]
        if ($call.Member.Value -ne 'new' -and @($helperType.GetMethods() | Where-Object { $_.IsStatic -and $_.Name -eq $call.Member.Value }).Count -eq 0) {
            throw "Missing public helper at $($call.Extent.Text)"
        }
    }
}
$cropValidation=[DashboardNative]::ValidateCrop()
$fontValidation=[DashboardNative]::ValidateExpectedFont()
$presentationValidation=[DashboardNative]::ValidatePresentation()
if ($ValidateOnly) {
    Write-Output "PASS: PowerShell parser/public helper call sites, declaration extraction, combined C# compilation; $cropValidation; $fontValidation; $presentationValidation"
    Write-Output 'VALIDATION ONLY: local expected-contract memory DC/font lifecycle, synthetic neutral/error ink checks and pure crop helpers only; no target EXE lookup/build, mutex, GUI, target geometry/font/PrintWindow, fixture, mock, network or acceptance tests run'
    return
}

$session=[Diagnostics.Process]::GetCurrentProcess().SessionId
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$mutexName="Local\AIRelay.$sid.$session.v1"
$child=$null; $childHandle=[IntPtr]::Zero; $childStart=$null; $ownedPid=0
$window=[IntPtr]::Zero; $hostWindow=[IntPtr]::Zero; $mock=$null; $temp=$null; $failure=$null
$previousDpi=[IntPtr]::Zero; $stage='preflight'
$overall=[Diagnostics.Stopwatch]::StartNew()
$checks=[Collections.Generic.List[object]]::new()
$samples=[Collections.Generic.List[object]]::new()
$shots=[Collections.Generic.List[object]]::new()
$limitations=[Collections.Generic.List[string]]::new()
$controls=@{}
$cleanupErrors=[Collections.Generic.List[string]]::new()
$expectedSuccess=0; $expectedError=0
$culture=[Globalization.CultureInfo]::InvariantCulture
function Record([string]$Name,[object]$Value) {
    $checks.Add([pscustomobject]@{ Check=$Name; Value=$Value; Seconds=[Math]::Round($overall.Elapsed.TotalSeconds,3) })
    Write-Host ("CHECK {0}: {1}" -f $Name,($Value | ConvertTo-Json -Depth 7 -Compress))
}
function Assert-NoOtherProcess {
    $others=@(Get-Process -Name 'ai-relay*' -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $ownedPid })
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
    $h=if ($controls.ContainsKey($Id)) { $controls[$Id] } else { [DashboardNative]::FindControl($window,$Id,[uint32]$ownedPid) }
    # Operational lookups stay strict; only a nonzero owner-checked HWND is pinned.
    [ProxyUiSmoke]::CheckOwner($h,[uint32]$ownedPid)
    $controls[$Id]=$h
    return $h
}
function Text([int]$Id) { [ModernNative]::BoundedText((Control $Id),[uint32]$ownedPid,4096) }
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
    Ensure-Reachable $h
    [ModernNative]::Click($h,[uint32]$ownedPid)
    Fence
}
function Select-Page([int]$Index,[int]$Expected) {
    [ModernNative]::ClickTab($childHandle,[uint32]$ownedPid,(Control 2100),$Index)
    Fence
    Wait-State "page $Index / $Expected visible" { (Message 2100 0x130B) -eq $Index -and (Visible $Expected) }
}
function Fence {
    # Drain asynchronous native Tab/category notifications without inventing them.
    $before=[regex]::Match((Text 2354),'(?:^|;)fence=(\d+)(?:;|$)')
    if (-not $before.Success) { throw 'Missing owner-thread fence metadata 2354' }
    $sequence=[uint64]$before.Groups[1].Value
    [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,2397)
    Wait-State 'owner-thread queue fence' {
        $now=[regex]::Match((Text 2354),'(?:^|;)fence=(\d+)(?:;|$)')
        $now.Success -and [uint64]$now.Groups[1].Value -gt $sequence
    }
    if ((Text 2354) -notmatch '(?:^|;)rejected=0(?:;|$)') { throw 'An owner-thread command was rejected' }
    if ([ProxyUiSmoke]::Dialog([uint32]$ownedPid) -ne [IntPtr]::Zero) { throw 'Unexpected owned modal dialog; no automatic confirmation sent' }
}
function Scroll-Overview([int]$Action) {
    [void](Message 2400 0x115 ([uint32]$Action))
    Fence
}
function Ensure-Reachable([IntPtr]$Handle) {
    Assert-Child; Fence
    if (-not [ProxyUiSmoke]::IsWindowVisible($Handle)) { throw 'Refusing input to hidden control' }
    if ([StatusBarNative]::GetParent($Handle) -ne $hostWindow) { return }
    for ($attempt=0;$attempt -lt 32;$attempt++) {
        $r=[ModernNative]::Bounds($Handle,$hostWindow,[uint32]$ownedPid)
        $client=[StatusBarNative]::Client($hostWindow,[uint32]$ownedPid)
        if ($r.Left -lt 0 -or $r.Right -gt $client.Width -or $r.Height -gt $client.Height) { throw "Control cannot fit actual viewport before input: $r client=$client" }
        if ($r.Top -ge 0 -and $r.Bottom -le $client.Height) { return }
        $old=[ModernNative]::Scroll($hostWindow,[uint32]$ownedPid).Position
        if ($r.Top -lt 0) { Scroll-Overview 0 } else { Scroll-Overview 1 }
        if ([ModernNative]::Scroll($hostWindow,[uint32]$ownedPid).Position -eq $old) { break }
    }
    throw "Control not reachable through bounded native scrolling: ID=$([ClarityNative]::GetDlgCtrlID($Handle))"
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
    $labels=if ($rates.Success) { @("请求 $($rates.Groups[1].Value)","错误 $($rates.Groups[2].Value)","上传 $($rates.Groups[3].Value)","下载 $($rates.Groups[4].Value)") }
        else { @('请求 未知','错误 未知','上传 未知','下载 未知') }
    return [pscustomobject]@{
        Labels=$labels; UnknownReasonRow=($null -eq $r -and $Value -notmatch '尚无快照')
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
function Assert-Layout([string]$Name,[ValidateSet('any','top','bottom')][string]$Endpoint='any') {
    Assert-Child; Fence
    if ((Message 2100 0x130B) -ne 0) { throw "$Name layout requires the actual overview page" }
    $dpi=[ClarityNative]::GetDpiForWindow($window)
    if ($dpi -eq 0) { throw 'Missing native window DPI' }
    $client=[StatusBarNative]::Client($window,[uint32]$ownedPid)
    $outer=[StatusBarNative]::Outer($window,[uint32]$ownedPid)
    $work=[WindowSmoke]::Work($window,[uint32]$ownedPid)
    $hostRect=[StatusBarNative]::InParent($hostWindow,$window,[uint32]$ownedPid)
    $hostClient=[StatusBarNative]::Client($hostWindow,[uint32]$ownedPid)
    $hostScreen=[DashboardNative]::ClientOnScreen($hostWindow,[uint32]$ownedPid)
    $clip=[ModernNative]::ClientBounds($hostWindow,$window,[uint32]$ownedPid)
    $scroll=[ModernNative]::Scroll($hostWindow,[uint32]$ownedPid)
    $tree=[DashboardNative]::Descendants($window,[uint32]$ownedPid)
    if ($outer.Left -lt $work.Left -or $outer.Top -lt $work.Top -or $outer.Right -gt $work.Right -or $outer.Bottom -gt $work.Bottom) { throw "$Name exceeds physical monitor work area" }
    $footerSet=@($tree | Where-Object {
        if ([StatusBarNative]::GetParent($_) -ne $window -or [ProxyUiSmoke]::Class($_) -ne 'Static' -or -not [ProxyUiSmoke]::IsWindowVisible($_)) { return $false }
        $r=[StatusBarNative]::InParent($_,$window,[uint32]$ownedPid)
        $r.Left -eq 0 -and $r.Right -eq $client.Width -and $r.Bottom -eq $client.Height -and $r.Height -eq [ModernNative]::Scale(40,$dpi)
    })
    if ($footerSet.Count -ne 1) { throw "$Name expected exactly one fixed 40-DIP footer" }
    $footer=$footerSet[0]; $footerRect=[StatusBarNative]::InParent($footer,$window,[uint32]$ownedPid)
    if ([StatusBarNative]::GetParent($hostWindow) -ne $window -or $hostRect.Left -ne 0 -or $hostRect.Right -ne $client.Width -or
        $hostRect.Top -ne [ModernNative]::Scale(44,$dpi) -or $hostRect.Bottom -ne $footerRect.Top -or
        ([ProxyUiSmoke]::GetWindowLongW($hostWindow,-20) -band 0x10000) -eq 0 -or ([ProxyUiSmoke]::GetWindowLongW($hostWindow,-16) -band 0x02000000) -eq 0) {
        throw "$Name fixed-header/footer and clipping-host contract failed"
    }
    $virtualHeight=[Math]::Max($client.Height,[ModernNative]::Scale(820,$dpi))
    $documentBottom=$virtualHeight-[ModernNative]::Scale(40+16,$dpi)
    if ($scroll.Min -ne 0 -or $scroll.Position -lt 0 -or $scroll.Position -gt $scroll.Limit -or $scroll.Page -ne $hostClient.Height -or
        [Math]::Abs(($scroll.Max+1)-($virtualHeight-[ModernNative]::Scale(44+40,$dpi))) -gt 1) { throw "$Name native scroll range/page differs from actual viewport/virtual document" }
    if (($Endpoint -eq 'top' -and $scroll.Position -ne $scroll.Min) -or ($Endpoint -eq 'bottom' -and $scroll.Position -ne $scroll.Limit)) { throw "$Name native $Endpoint endpoint not reached" }
    # Recurse for discovery, but compare only siblings, never a host with its child.
    $fixed=[Collections.Generic.List[object]]::new(); $rects=[Collections.Generic.List[object]]::new()
    $metadata=[ModernNative]::OverviewMetadata($window,$hostWindow,(Control 2107),[uint32]$ownedPid)
    foreach ($h in $tree) {
        if (-not [ProxyUiSmoke]::IsWindowVisible($h)) { continue }
        $parent=[StatusBarNative]::GetParent($h)
        if ($parent -eq $window) {
            if ($h -eq $hostWindow) { continue }
            $r=[ModernNative]::Bounds($h,$window,[uint32]$ownedPid)
            if ($r.Width -le 0 -or $r.Height -le 0 -or $r.Left -lt 0 -or $r.Top -lt 0 -or $r.Right -gt $client.Width -or $r.Bottom -gt $client.Height -or
                ($h -ne $footer -and $r.Bottom -gt $hostRect.Top)) { throw "$Name fixed main child outside header/footer: ID=$([ClarityNative]::GetDlgCtrlID($h)) rect=$r" }
            $fixed.Add([pscustomobject]@{ Handle=$h; ID=[ClarityNative]::GetDlgCtrlID($h); Class=[ProxyUiSmoke]::Class($h); Rect=$r; NativeScreenBounds=[StatusBarNative]::Outer($h,[uint32]$ownedPid) })
        } elseif ($parent -eq $hostWindow) {
            $id=[ClarityNative]::GetDlgCtrlID($h)
            # 2107 alone is a zero-area read-only metadata HWND; WS_VISIBLE is allowed.
            if ($id -eq 2107) { continue }
            $raw=[ModernNative]::Bounds($h,$hostWindow,[uint32]$ownedPid)
            $document=[ModernNative]::Document($raw,$scroll.Position,$dpi)
            $inMain=[ModernNative]::Bounds($h,$window,[uint32]$ownedPid)
            $screen=[StatusBarNative]::Outer($h,[uint32]$ownedPid)
            $effective=[ModernNative]::Intersect($inMain,$clip)
            if ($raw.Width -le 0 -or $raw.Height -le 0 -or $document.Left -lt 0 -or $document.Right -gt $hostClient.Width -or $document.Top -lt 0 -or $document.Bottom -gt $documentBottom+1) {
                throw "$Name invalid complete document HWND: ID=$id document=$document actualHostClient=$hostClient"
            }
            if ($screen.Left-$hostScreen.Left -ne $raw.Left -or $screen.Top-$hostScreen.Top -ne $raw.Top) { throw "$Name inconsistent absolute/host-client coordinates: ID=$id" }
            if ($effective.Width -gt 0 -and $effective.Height -gt 0 -and ($effective.Top -lt $hostRect.Top -or $effective.Bottom -gt $footerRect.Top)) { throw "$Name effective page content covers fixed chrome: ID=$id" }
            $rects.Add([pscustomobject]@{ Handle=$h; ID=$id; Class=[ProxyUiSmoke]::Class($h); Document=$document; HostClientBounds=$raw; MainClientBounds=$inMain; NativeScreenBounds=$screen; Effective=$effective; ClippedByViewport=($effective.Width -lt $raw.Width -or $effective.Height -lt $raw.Height) })
        }
    }
    if ($rects.Count -lt 8 -or $fixed.Count -lt 2) { throw "$Name incomplete recursive main/viewport discovery; no vacuous layout pass" }
    foreach ($group in @(@{ Items=$fixed; Field='Rect' },@{ Items=$rects; Field='Document' })) {
        $items=$group.Items; $field=$group.Field
        for ($a=0;$a -lt $items.Count;$a++) { for ($b=$a+1;$b -lt $items.Count;$b++) {
            $overlap=[ModernNative]::Intersect($items[$a].$field,$items[$b].$field)
            if ($overlap.Width -gt 0 -and $overlap.Height -gt 0) { throw "$Name sibling overlap: ID=$($items[$a].ID) vs ID=$($items[$b].ID)" }
        } }
    }
    foreach ($anchor in @(@{Id=2108;Top=108;Height=100},@{Id=2109;Top=282;Height=$null},@{Id=2102;Top=220;Height=36},@{Id=2103;Top=220;Height=36},@{Id=2104;Top=220;Height=36},@{Id=2105;Top=220;Height=36},@{Id=2106;Top=220;Height=36})) {
        $matches=@($rects | Where-Object { $_.ID -eq $anchor.Id })
        if ($matches.Count -ne 1 -or [Math]::Abs($matches[0].Document.Top-[ModernNative]::Scale($anchor.Top,$dpi)) -gt 1) { throw "$Name missing/stale document anchor: ID=$($anchor.Id) scroll=$($scroll.Position)" }
        if ($null -ne $anchor.Height -and [Math]::Abs($matches[0].Document.Height-[ModernNative]::Scale($anchor.Height,$dpi)) -gt 1) { throw "$Name wrong native control height: ID=$($anchor.Id)" }
    }
    $title=@($rects | Where-Object { $_.Class -eq 'Static' -and [ModernNative]::BoundedText($_.Handle,[uint32]$ownedPid,4096) -eq '运行概览' })
    if ($title.Count -ne 1 -or [Math]::Abs($title[0].Document.Top-[ModernNative]::Scale(64,$dpi)) -gt 1) { throw "$Name missing overview document heading" }
    $rightMargins=[Collections.Generic.List[object]]::new()
    foreach ($id in @(2108,2109)) {
        $full=[StatusBarNative]::InParent((Control $id),$hostWindow,[uint32]$ownedPid)
        $margin=[ModernNative]::PageRightMargin($hostClient.Width,$full,$dpi)
        if ($null -ne $margin.Failure -or [Math]::Abs($full.Left-[ModernNative]::Scale(24,$dpi)) -gt 1) { throw "$Name wrong 24-DIP margin against ACTUAL host client: ID=$id proof=$($margin | ConvertTo-Json -Compress)" }
        $rightMargins.Add([pscustomobject]@{ ID=$id; FullHostClientBounds=$full; Proof=$margin })
    }
    $dash=@($rects | Where-Object { $_.ID -eq 2109 })[0]
    if ([Math]::Abs($dash.Document.Bottom-$documentBottom) -gt 1) { throw "$Name dashboard does not end 16 DIP before virtual footer" }
    $tail=[ModernNative]::PageTail(0,$client.Height,$clip,$footerRect,$scroll,$dash.MainClientBounds,$dash.Document.Bottom,$dpi)
    if ($null -ne $tail.Failure) { throw "$Name complete native dashboard tail failed: $($tail.Failure)" }
    $canPlot=$dash.Document.Width -ge [ModernNative]::Scale(696,$dpi) -and $dash.Document.Height -ge [ModernNative]::Scale(350,$dpi)
    Record "$Name measured native HWND/document/absolute-screen geometry (NOT painter plot rectangles)" @{ DPI=$dpi; Outer=$outer; Work=$work; Client=$client; Host=$hostRect; ActualHostClient=$hostClient; HostScreenClient=$hostScreen; Scroll=$scroll; DocumentBottom=$documentBottom; Footer=$footerRect; Dashboard=$dash; CanRenderTwoPlots=$canPlot; Tail=$tail; RightMargins=$rightMargins.ToArray(); Metadata2107=$metadata; Fixed=$fixed.ToArray(); PageChildren=$rects.ToArray() }
}
function Assert-OverviewEndpoints([string]$Name) {
    Scroll-Overview 6
    Assert-Layout "$Name-top" 'top'
    Scroll-Overview 7
    Assert-Layout "$Name-bottom" 'bottom'
    Scroll-Overview 6
}
function Resize-Overview([ValidateSet('minimum','default','expanded')][string]$Size) {
    Assert-Child
    $dpi=[ClarityNative]::GetDpiForWindow($window)
    $work=[WindowSmoke]::Work($window,[uint32]$ownedPid)
    $sizes=@{ minimum=@(800,560); default=@(800,680); expanded=@(1000,820) }; $requested=$sizes[$Size]
    $requestedW=[ModernNative]::Scale($requested[0],$dpi); $requestedH=[ModernNative]::Scale($requested[1],$dpi)
    $w=[Math]::Min($work.Right-$work.Left,$requestedW); $h=[Math]::Min($work.Bottom-$work.Top,$requestedH)
    if ($w -le 0 -or $h -le 0) { throw 'Invalid native monitor work area' }
    $clamped=$w -ne $requestedW -or $h -ne $requestedH
    if ($clamped) {
        $limit="$Size requested outer $($requested[0])x$($requested[1]) DIP was work-area-clamped to ${w}x${h} physical pixels at DPI $dpi; exact requested size NOT exercised"
        if (-not $limitations.Contains($limit)) { $limitations.Add($limit) }
        Record 'LIMIT work-area clamp (not an exact-size pass)' $limit
    }
    [DashboardNative]::Resize($window,[uint32]$ownedPid,$work.Left,$work.Top,$w,$h)
    Wait-State 'physical outer resize and absolute placement completed' { $r=[StatusBarNative]::Outer($window,[uint32]$ownedPid); $r.Left -eq $work.Left -and $r.Top -eq $work.Top -and $r.Width -eq $w -and $r.Height -eq $h }
    Fence
    Start-Sleep -Milliseconds 150
    Record "$Size requested outer DIP / measured physical size" @{ RequestedDIP=$requested; Actual=[StatusBarNative]::Outer($window,[uint32]$ownedPid); Work=$work; DPI=$dpi; WorkAreaClamped=$clamped; InteractiveMinTrack='Not exercised' }
}
function Capture([string]$Name,[IntPtr]$Handle,[switch]$Outer) {
    Assert-Child
    if ($Handle -ne $window -or -not $Outer) { throw 'Only whole main-window outer PrintWindow is allowed; child printing can inherit parent clipping' }
    if (-not [ProxyUiSmoke]::IsWindowVisible($Handle)) { throw "Hidden PrintWindow target $Name" }
    $native=if ($Outer) { [StatusBarNative]::Outer($Handle,[uint32]$ownedPid) } else { [StatusBarNative]::Client($Handle,[uint32]$ownedPid) }
    # Preserve the whole outer render, including title/non-client frame and the
    # actual viewport clipping. A later crop requires COMPLETE native containment.
    $path=Join-Path $output "$Name-PRINTWINDOW-OFFSCREEN.png"
    try { $shot=[ModernNative]::Capture($Handle,[uint32]$ownedPid,$path,(-not $Outer),(New-Object StatusBarNative+Rect)) }
    catch {
        $captureError=$_.Exception
        $exceptionDetails=[DashboardNative]::ExceptionDetails($captureError)
        $analysis=$null; $analysisError=$null
        if ([IO.File]::Exists($path)) {
            try { $analysis=[DashboardNative]::AnalyzePng($path) }
            catch { $analysisError=[DashboardNative]::ExceptionDetails($_.Exception) }
        }
        Record 'FAIL PrintWindow OFFSCREEN; retained PNG is NOT an accepted screenshot' @{ Path=$path; Exception=$exceptionDetails; SavedPngAnalysis=$analysis; AnalysisError=$analysisError; ExpectedNativeSize=$native }
        throw [InvalidOperationException]::new(("PrintWindow OFFSCREEN failed: {0}`n{1}`nRead-only saved PNG analysis: {2}`nPNG analysis error: {3}" -f $path,$exceptionDetails,($analysis | ConvertTo-Json -Compress),$analysisError),$captureError)
    }
    if ($shot.Width -ne $native.Width -or $shot.Height -ne $native.Height) { throw 'PrintWindow dimensions differ from complete native HWND' }
    $scope='Whole main-window outer PrintWindow OFFSCREEN render WITH viewport clipping; not automatically a complete dashboard or physical visibility proof'
    $shots.Add([pscustomobject]@{ Name=$Name; CaptureKind='PrintWindow OFFSCREEN, NOT desktop capture'; Scope=$scope; NativeScreenBounds=[StatusBarNative]::Outer($Handle,[uint32]$ownedPid); DPI=[ClarityNative]::GetDpiForWindow($Handle); Shot=$shot })
    return $shot
}
function Capture-OverviewEndpoints([string]$Name) {
    Scroll-Overview 6
    [void](Capture "$Name-whole-top" $window -Outer)
    Scroll-Overview 7
    [void](Capture "$Name-whole-bottom" $window -Outer)
    Scroll-Overview 6
}
function Capture-Dashboard([string]$Name,[IntPtr]$Dashboard) {
    Assert-Child
    Ensure-Reachable $Dashboard
    $geometry=[DashboardNative]::DashboardCropBounds($window,$hostWindow,$Dashboard,[uint32]$ownedPid)
    $whole=Capture "$Name-whole" $window -Outer
    $path=Join-Path $output "$Name-WHOLEWINDOWOFFSCREEN-COMPLETEVIEWPORT-CONTAINEDCROP.png"
    Assert-Child
    try { $shot=[DashboardNative]::CropDashboard($whole.Path,$path,$geometry,$window,$hostWindow,$Dashboard,[uint32]$ownedPid) }
    catch {
        $cropError=$_.Exception; $details=[DashboardNative]::ExceptionDetails($cropError)
        $analysis=$null; $analysisError=$null
        if ([IO.File]::Exists($path)) {
            try { $analysis=[DashboardNative]::AnalyzePng($path) }
            catch { $analysisError=[DashboardNative]::ExceptionDetails($_.Exception) }
        }
        Record 'FAIL wholewindowoffscreen-completeviewport-containedcrop (NOT accepted)' @{ SourceWholePng=$whole.Path; CropPath=$path; Geometry=$geometry; Exception=$details; SavedPngAnalysis=$analysis; AnalysisError=$analysisError }
        throw [InvalidOperationException]::new(("Complete viewport-contained dashboard crop failed: {0}`n{1}" -f $path,$details),$cropError)
    }
    $shots.Add([pscustomobject]@{ Name=$Name; CaptureKind='wholewindowoffscreen-completeviewport-containedcrop'; Scope='Exact full native dashboard rectangle in a whole-window OFFSCREEN PrintWindow render, entirely inside host client; NOT independent child PrintWindow or desktop capture'; SourceWholePng=$whole.Path; Geometry=$geometry; DPI=$geometry.Dpi; Shot=$shot })
    return $shot
}
function Assert-DashboardPixels([string]$Name,[bool]$Dark) {
    $dashboard=Control 2109
    $dpi=[ClarityNative]::GetDpiForWindow($dashboard)
    $native=[StatusBarNative]::Client($dashboard,[uint32]$ownedPid)
    if ($native.Width -lt [ModernNative]::Scale(696,$dpi) -or $native.Height -lt [ModernNative]::Scale(350,$dpi)) {
        $limit="$Name real dashboard is below 696x350 DIP and uses text fallback; legend/units/curve pixel assertions NOT run, no mock plot manufactured"
        $limitations.Add($limit); Record 'LIMIT real dashboard fallback' $limit
        Capture-OverviewEndpoints "$Name-text-fallback"
        return $null
    }
    $full=[StatusBarNative]::Outer($dashboard,[uint32]$ownedPid)
    $hostClient=[StatusBarNative]::Client($hostWindow,[uint32]$ownedPid)
    if ($full.Width -gt $hostClient.Width) { throw "$Name complete dashboard width exceeds actual host client; not a valid vertical-clipping skip" }
    if ($full.Height -gt $hostClient.Height) {
        $limit="$Name complete native dashboard $($full.Width)x$($full.Height) physical pixels cannot fit actual viewport $($hostClient.Width)x$($hostClient.Height); only complete-PNG pixel acceptance is skipped, NOT a text fallback or a full-image pass"
        $limitations.Add($limit)
        Record 'LIMIT full dashboard exceeds host client (native geometry checks retained)' @{ Reason=$limit; DashboardScreen=$full; HostClient=$hostClient; FullImagePixels='NOT exercised; source full-memory drawing/geometry tests are separate and not run by this script' }
        Capture-OverviewEndpoints "$Name-parent-clipped"
        return $null
    }
    for ($attempt=0;$attempt -lt 3;$attempt++) {
        $before=Observe
        $shot=Capture-Dashboard "$Name-attempt-$attempt" $dashboard
        $current=Observe
        if (($before.Labels -join '|') -cne ($current.Labels -join '|') -or $before.UnknownReasonRow -ne $current.UnknownReasonRow) { continue }
        $proof=[DashboardNative]::Inspect($shot.Path,$dashboard,[uint32]$ownedPid,$Dark,[string[]]$current.Labels,$current.UnknownReasonRow)
        foreach ($p in $proof) {
            if ($p.Role -eq 'badge') { [DashboardNative]::RequireBadgeInk($p,($p.Name -eq 'card-1-badge' -and $current.Errors -gt 0)) }
            if ($p.Role -in @('value','title','units','legend-text') -and $p.Ink -lt 4) { throw "No rough theme-text ink (not a glyph/value identification): $($p.Name)" }
            if (($p.Role -eq 'legend-blue' -and $p.Blue -lt 4) -or ($p.Role -eq 'legend-red' -and $p.Red -lt 4) -or ($p.Role -eq 'legend-neutral' -and $p.NeutralInk -lt 4)) { throw "No expected compact series glyph ink: $($p.Name)" }
            if ($p.Role -in @('units','legend-text') -and $p.ExpectedContractTextWidth -gt $p.ExpectedLayoutTextBudget) { throw "Harness expected painter-font contract text exceeds expected inspection budget: $($p.Name); NOT a target-font measurement or painter-rectangle observation" }
            if ($p.Role -eq 'plot' -and $null -ne $current.Rates -and $p.Colored -lt 4) { throw "No real sampled curve color in expected plot region: $($p.Name)" }
        }
        Record "$Name wholewindowoffscreen-completeviewport-containedcrop / rough regions vs exact WM_GETTEXT" @{ Screenshot=$shot; Regions=$proof; MetadataBefore=$before.Text; MetadataAfter=$current.Text; ExpectedCompactLabels=$current.Labels; ExpectedTitleUnits=@('区间均值 · 次/s','区间均值 · B/s'); UnknownReasonRow=$current.UnknownReasonRow; CurveColorAsserted=($null -ne $current.Rates); RegionSource='Actual complete viewport-contained PNG/HWND/DPI; expected regions use harness-owned Font::new contract widths, NOT target-HFONT or internal-painter-rectangle measurements'; WidthScope=[DashboardNative]::ExpectedFontWidthScope; TargetFontCheck='Read-only WM_GETFONT nonzero only; target assignment observed, identity NOT verified, handle never selected into harness DC or changed'; OCR='NOT performed; color/ink presence cannot identify labels, units or numerical glyphs, prove text fit, or establish physical visibility' }
        return $shot
    }
    throw 'Rate labels/unknown-state changed during all three bounded PrintWindow attempts; no stable region assertion made'
}
function Assert-DashboardSurfacePair([string]$Name,[object]$Light,[object]$Dark) {
    if ($null -eq $Light -or $null -eq $Dark) {
        Record "$Name SKIP full-dashboard light/dark surface comparison" 'A complete viewport-contained crop was unavailable (host clipping or real text fallback); see limitations. Partial whole-window renders are not a full-image pass.'
        return
    }
    if ($Light.MeanLuminance -lt 150 -or $Dark.MeanLuminance -gt 125 -or $Light.MeanLuminance-$Dark.MeanLuminance -lt 60) { throw "$Name whole-window OFFSCREEN complete dashboard crop surfaces did not change distinctly (high contrast requires separate acceptance)" }
    Record "$Name complete viewport-contained crop light/dark difference (NOT desktop visibility)" @{ CaptureKind='wholewindowoffscreen-completeviewport-containedcrop'; Light=$Light.MeanLuminance; Dark=$Dark.MeanLuminance; LightPath=$Light.Path; DarkPath=$Dark.Path }
}
function Theme([int]$Index) {
    Select-Page 3 2039
    Scroll-Overview 6
    if ([ProxyUiSmoke]::Class((Control 2039)) -ne 'SysTabControl32') { throw '2039 must be the native six-item settings segment' }
    Ensure-Reachable (Control 2039)
    [ModernNative]::ClickTab($childHandle,[uint32]$ownedPid,(Control 2039),5)
    Fence
    Wait-State 'appearance category selected / theme segment visible' { (Message 2039 0x130B) -eq 5 -and (Visible 2030) }
    if ([ProxyUiSmoke]::Class((Control 2030)) -ne 'SysTabControl32') { throw '2030 must be native segmented SysTabControl32' }
    $frozen=Text 2109
    Ensure-Reachable (Control 2030)
    [ModernNative]::ClickTab($childHandle,[uint32]$ownedPid,(Control 2030),$Index)
    Fence
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
    $stage='wait for complete owned control tree'
    # The main HWND exists before children are created/adopted. Do not use strict
    # Control/Text or cache any startup probe until the required tree is complete.
    Wait-State 'owned viewport and complete adopted smoke-control tree' {
        $candidateHost=[DashboardNative]::FindControl($window,2400,[uint32]$ownedPid)
        if ($candidateHost -eq [IntPtr]::Zero -or [StatusBarNative]::GetParent($candidateHost) -ne $window) { return $false }
        foreach ($id in @(2100,2102,2103,2104,2105,2106,2107,2108,2109,2030,2039,2300,2354)) {
            $candidate=[DashboardNative]::FindControl($window,$id,[uint32]$ownedPid)
            if ($candidate -eq [IntPtr]::Zero) { return $false }
            $expectedParent=if ($id -eq 2100) { $window } else { $candidateHost }
            if ([StatusBarNative]::GetParent($candidate) -ne $expectedParent) { return $false }
        }
        return $true
    }
    $hostWindow=Control 2400
    if ([StatusBarNative]::GetParent($hostWindow) -ne $window) { throw '2400 must be the actual direct clipping/scrolling host' }
    Wait-State 'first actual metrics snapshot' { (Text 2109).Contains('本次程序会话：累计请求') }
    $first=Observe
    if ($first.Text.Contains('首个样本')) {
        if ($null -ne $first.Rates) { throw 'First sample invented zero rates' }
        Record 'first sample unknown, not zero' $first
    } else {
        $limitations.Add('First <=1s unknown window already elapsed; initial unknown display NOT exercised and not manufactured')
        Record 'SKIP first <=1s unknown window (already elapsed; not manufactured)' $first.Text
    }
    Wait-State 'saved private config loaded / stopped' { (Enabled 2102) -and -not (Enabled 2103) -and (Enabled 2104) }
    Record 'pinned exact child / EXE identity' @{ PID=$ownedPid; Start=$childStart.ToString('o'); Exe=$exe; SHA256=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash; LastWriteUTC=[IO.File]::GetLastWriteTimeUtc($exe).ToString('o') }
    if ([ProxyUiSmoke]::Class((Control 2109)) -ne 'Static' -or ([ProxyUiSmoke]::GetWindowLongW((Control 2109),-16) -band 31) -ne 13 -or
        [StatusBarNative]::GetParent((Control 2109)) -ne $hostWindow) { throw 'Expected 2109 SS_OWNERDRAW STATIC under actual host 2400' }
    [void][ModernNative]::OverviewMetadata($window,$hostWindow,(Control 2107),[uint32]$ownedPid)
    Resize-Overview 'minimum'
    Assert-OverviewEndpoints 'minimum-800x560-or-workarea-clamp'
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
        Assert-OverviewEndpoints "$name-minimum-800x560-or-clamp"
        if ($index -eq 0) { Record 'system theme selected by native segment; retained real session' $current; continue }
        $shot=Assert-DashboardPixels "dashboard-$name-minimum-full" ($index -eq 2)
        if ($index -eq 1) { $light=$shot } else { $dark=$shot }
    }
    Assert-DashboardSurfacePair 'minimum' $light $dark
    foreach ($size in @('default','expanded')) {
        Resize-Overview $size
        $light=$null; $dark=$null
        foreach ($index in @(1,2)) {
            $name=@('system','light','dark')[$index]
            Theme $index
            Assert-Totals (Observe)
            Assert-Buttons $true
            Assert-OverviewEndpoints "$name-$size-or-workarea-clamp"
            [void](Capture "overview-$name-$size-top" $window -Outer)
            $shot=Assert-DashboardPixels "dashboard-$name-$size-full" ($index -eq 2)
            if ($index -eq 1) { $light=$shot } else { $dark=$shot }
            Scroll-Overview 7
            [void](Capture "overview-$name-$size-bottom" $window -Outer)
            Scroll-Overview 6
        }
        Assert-DashboardSurfacePair $size $light $dark
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
    Assert-OverviewEndpoints 'stopped-expanded-or-workarea-clamp'
    [void](Capture 'overview-dark-stopped' $window -Outer)
    [void](Assert-DashboardPixels 'dashboard-dark-stopped-real-zero-full' $true)
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
    if ($null -ne $failure) { $detail=[pscustomobject]@{ Stage=$stage; Message=$failure.Exception.Message; ExceptionDetails=[DashboardNative]::ExceptionDetails($failure.Exception); Position=$failure.InvocationInfo.PositionMessage; Stack=$failure.ScriptStackTrace } }
    $report=[pscustomobject]@{
        Passed=($null -eq $failure -and $cleanupErrors.Count -eq 0); Complete=$false; Exe=$exe; PID=$ownedPid
        Failure=$detail; CleanupErrors=@($cleanupErrors.ToArray()); Checks=@($checks.ToArray()); ObservedLimitations=@($limitations.ToArray())
        ActualSamples=@($samples.ToArray()); Screenshots=@($shots.ToArray()); Seconds=$overall.Elapsed.TotalSeconds
        CaptureKind='PrintWindow OFFSCREEN only; NOT desktop capture or independent child PrintWindow. Full dashboard images use wholewindowoffscreen-completeviewport-containedcrop: exact native full-HWND rectangle inside actual host client and whole outer bounds, checked before/after printing. Oversized dashboards retain top/bottom whole-window renders and skip complete-image pixel acceptance.'
        GeometryScope='Native HWND rectangles, actual host-client width, absolute screen coordinates, native scroll range and document transforms measured. Chart-region text budgets use a harness-owned expected Font::new contract at actual DPI, NOT target-HFONT measurement, target font-identity verification, exported painter rectangles or OCR/actual-fit proof.'
        WidthScope=[DashboardNative]::ExpectedFontWidthScope
        Limits='Complete=false: this bounded smoke is not full acceptance. Current monitor DPI only; no physical visibility/occlusion/residual, physical min-track/input/tray, multi-monitor/high-contrast/screen-reader acceptance. Outer minimum 800x560, default 800x680 and expanded 1000x820 DIP are work-area-clamped and reported; clamped sizes are not exact-size passes. Dashboard larger than its viewport skips only full-PNG pixel acceptance, retains native geometry and top/bottom whole-window renders, and is NOT called text fallback. Real text fallback applies only below 696x350 DIP, never replaced with a mock plot. Crops never intersect/shrink a partial HWND, resize or stitch; dimensions include native non-client origin mapping and unchanged magenta/color guard. Target WM_GETFONT is checked only for nonzero assignment; its opaque cross-process HFONT is never selected, inspected or replaced. Text-width budgets use a temporary harness-owned memory DC/CreateFontW matching the expected Font::new contract, not observed target font realization or painter rectangles. Local lifecycle self-checks do not verify target GDI resources or actual fit. Pixel checks retain rough badge/text/series ink/color thresholds, not unit/label/value glyph identity or compact layout correctness; target-native painter/font/geometry/GDI/printing tests are separate. No induced unknown gap/reset/saturation or 120s/120-point retention acceptance; initial <=1s may be skipped. Copy/directory availability only (no clipboard/Explorer). Short GDI bounds, not long stability. No Cargo, old script execution, real API/key, Startup/firewall/system proxy changes, global input or IPC to another instance.'
    }
    [IO.File]::WriteAllText((Join-Path $output 'results.json'),($report | ConvertTo-Json -Depth 14),[Text.UTF8Encoding]::new($true))
    Write-Output "Overview dashboard artifacts: $output"
}
if ($null -ne $failure) { throw $failure }
if ($cleanupErrors.Count -gt 0) { throw ($cleanupErrors -join '; ') }
Write-Output 'PASS (executed bounded checks only; Complete=false): real loopback counters/body bytes/rates, hidden freeze/catch-up, native themes/buttons and recursive viewport geometry, whole-window PrintWindow OFFSCREEN renders / fully-contained dashboard crops where available, short GDI bounds, owned-child exit and config unchanged. See results.json for skipped/clamped cases and non-acceptance limits.'
