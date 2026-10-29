# Windows PowerShell 5.1 x64, interactive Windows 10/11 desktop.
# Only this script's pinned child and loopback mock are used. Never runs Cargo.
# Screenshots are PrintWindow OFFSCREEN renders, not desktop/occlusion/residual proof.
# No clipboard, config Save, Startup, Explorer or Windows preference changes.
# Run -ValidateOnly first; the primary agent must exclusively own the later GUI run.
[CmdletBinding()]
param(
    [string]$ExePath,
    [switch]$ValidateOnly
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([IntPtr]::Size -ne 8) { throw 'Use 64-bit powershell.exe -NoProfile' }
if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $PSScriptRoot '..\target\debug\ai-relay.exe'
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
using System.Text;
using System.Threading.Tasks;
public static class ModernNative {
    [StructLayout(LayoutKind.Sequential)] public struct ComboInfo {
        public uint Size; public StatusBarNative.Rect Item,Button; public uint State;
        public IntPtr Combo,Edit,List;
    }
    public sealed class Shot {
        public string Path; public string CaptureKind="PrintWindow OFFSCREEN rendering (physical-pixel dimensions, NOT desktop capture)";
        public int Width,Height,Colors;
        public double MeanLuminance,BrightFraction,BrightTileFraction;
    }
    public sealed class PendingProof {
        public bool Seen; public int Attempts; public string[] Frozen;
        public int Date,Zone,Trend,Table,Export;
    }
    [DllImport("user32.dll")] static extern bool GetComboBoxInfo(IntPtr window,ref ComboInfo info);
    [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr window,IntPtr dc,uint flags);
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr window,uint relation);
    [DllImport("user32.dll")] static extern bool GetScrollInfo(IntPtr window,int bar,ref ScrollInfo info);
    [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr window,ref StatusBarNative.Point point);
    [DllImport("user32.dll")] static extern bool ScreenToClient(IntPtr window,ref StatusBarNative.Point point);
    [DllImport("user32.dll")] static extern IntPtr MonitorFromWindow(IntPtr window,uint flags);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern bool GetMonitorInfoW(IntPtr monitor,ref MonitorInfo info);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr window,IntPtr after,int x,int y,int width,int height,uint flags);
    [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr window,uint message,UIntPtr w,IntPtr l);
    [DllImport("user32.dll")] static extern int GetWindowRgn(IntPtr window,IntPtr region);
    [DllImport("gdi32.dll")] static extern IntPtr CreateRectRgn(int l,int t,int r,int b);
    [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr obj);
    [StructLayout(LayoutKind.Sequential)] struct MonitorInfo { public uint Size; public StatusBarNative.Rect Monitor,Work; public uint Flags; }
    [StructLayout(LayoutKind.Sequential)] public struct ScrollInfo {
        public uint Size,Mask; public int Min,Max; public uint Page; public int Position,Track;
        public int Limit { get { return Math.Max(Min,Max-(int)Math.Max(1u,Page)+1); } }
    }
    public sealed class ContrastProof {
        public string BackgroundRgb,ForegroundRgb,Scope;
        public double BackgroundLuminance,Contrast;
        public int Samples,InkSamples,SampleStep;
    }
    public sealed class MetadataProof {
        public IntPtr Handle; public StatusBarNative.Rect Bounds;
        public long Style; public int TextLength;
        public string Contract;
    }
    public sealed class EditProof {
        public IntPtr Handle; public bool Password;
        public long NativeStyle,Length,Selection,Margins,Mask,Limit;
        public string Text;
    }
    public sealed class TailProof {
        public bool AtBottom,StrictGap;
        public int Gap,Padding,Extent,ExpectedExtent,LastDocumentBottom;
        public string Failure;
    }
    public sealed class RightMarginProof {
        public int HostClientWidth,Right,ExpectedRight,Margin,ExpectedMargin;
        public string Failure;
    }
    public sealed class SegmentWidthProof {
        public int ID,HostClientWidth,Left,ExpectedLeft,Width,ExpectedWidth,Right,ExpectedRight,MaxWidthDIP;
        public string Failure;
    }
    [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr OpenProcess(uint access,bool inherit,uint pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr VirtualAllocEx(IntPtr process,IntPtr address,UIntPtr size,uint kind,uint protection);
    [DllImport("kernel32.dll")] static extern bool VirtualFreeEx(IntPtr process,IntPtr address,UIntPtr size,uint kind);
    [DllImport("kernel32.dll")] static extern bool ReadProcessMemory(IntPtr process,IntPtr address,byte[] bytes,UIntPtr size,out UIntPtr read);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr window,uint message,UIntPtr w,IntPtr l,uint flags,uint timeout,out UIntPtr result);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr window,uint message,UIntPtr w,StringBuilder l,uint flags,uint timeout,out UIntPtr result);
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
    public static int Scale(int dip,uint dpi) { return (int)((dip*(long)dpi+48)/96); }
    public static StatusBarNative.Rect ComboArrowRegion(StatusBarNative.Rect client,uint dpi) {
        // Match control_theme::combo_rects, not the native rcButton: the custom
        // closed ComboBox reserves 30 DIP for the arrow and 11 DIP text padding.
        if(dpi==0 || client.Width<2 || client.Height<2) throw new Exception("Invalid custom ComboBox client/DPI");
        int edge=Math.Min(Scale(1,dpi),Math.Min(client.Width/2,client.Height/2));
        return new StatusBarNative.Rect { Left=Math.Max(client.Right-Scale(30,dpi),client.Left+edge),
            Top=client.Top+edge,Right=client.Right-edge,Bottom=client.Bottom-edge };
    }
    public static string BoundedText(IntPtr window,uint pid,int maximum) {
        ProxyUiSmoke.CheckOwner(window,pid);
        if(maximum<1 || maximum>16384 || Message(window,pid,14,0,0)>maximum) throw new Exception("Metadata text exceeds bounded read contract");
        var text=new StringBuilder(maximum+1); UIntPtr copied;
        if(SendMessageTimeoutW(window,13,new UIntPtr((uint)text.Capacity),text,0x22,2000,out copied)==IntPtr.Zero)
            throw new Exception("Bounded metadata WM_GETTEXT timeout");
        if(copied.ToUInt64()>(ulong)maximum || copied.ToUInt64()!=(ulong)text.Length || Message(window,pid,14,0,0)>maximum)
            throw new Exception("Metadata read truncated or exceeded bound");
        return text.ToString();
    }
    static bool MetadataShape(int id,string kind,long style,StatusBarNative.Rect outer,StatusBarNative.Rect client) {
        // This is NOT a general exemption for empty controls or hidden pages.
        // STATIC SS_LEFT is display-only; SS_NOTIFY and WS_TABSTOP are forbidden.
        return id==2107 && String.Equals(kind,"Static",StringComparison.OrdinalIgnoreCase) &&
            (style&0x1001F)==0 && (style&0x100)==0 && outer.Width==0 && outer.Height==0 && client.Width==0 && client.Height==0;
    }
    public static MetadataProof OverviewMetadata(IntPtr main,IntPtr host,IntPtr metadata,uint pid) {
        ProxyUiSmoke.CheckOwner(main,pid); ProxyUiSmoke.CheckOwner(host,pid); ProxyUiSmoke.CheckOwner(metadata,pid);
        long style=unchecked((uint)ProxyUiSmoke.GetWindowLongW(metadata,-16));
        var outer=StatusBarNative.Outer(metadata,pid); var client=StatusBarNative.Client(metadata,pid);
        if(StatusBarNative.GetParent(metadata)!=host || !MetadataShape(ClarityNative.GetDlgCtrlID(metadata),ProxyUiSmoke.Class(metadata),style,outer,client))
            throw new Exception("2107 must be the zero-area, non-TabStop/non-notifying read-only STATIC metadata under viewport 2400");
        string text=BoundedText(metadata,pid,4096);
        foreach(string field in new[]{"实际监听：","上游：","本机 Base URL：","活动请求：","活动隧道：","入站连接：","累计请求：","错误：","上传正文／隧道：","下载正文／隧道：","程序会话时长：","本次程序会话累计"})
            if(!text.Contains(field)) throw new Exception("2107 legacy snapshot text missing field: "+field);
        return new MetadataProof { Handle=metadata,Bounds=StatusBarNative.InParent(metadata,main,pid),Style=style,TextLength=text.Length,
            Contract="ID 2107 only; STATIC SS_LEFT read-only text, no SS_NOTIFY/WS_TABSTOP; native window/client both 0x0; bounded <=4096 UTF-16 snapshot retained; no input or text mutation" };
    }
    public static ScrollInfo Scroll(IntPtr host,uint pid) {
        ProxyUiSmoke.CheckOwner(host,pid);
        var info=new ScrollInfo { Size=(uint)Marshal.SizeOf(typeof(ScrollInfo)),Mask=0x17 };
        if(!GetScrollInfo(host,1,ref info)) throw new Exception("Viewport GetScrollInfo failed");
        return info;
    }
    public static StatusBarNative.Rect Intersect(StatusBarNative.Rect a,StatusBarNative.Rect b) {
        int l=Math.Max(a.Left,b.Left),t=Math.Max(a.Top,b.Top);
        return new StatusBarNative.Rect { Left=l,Top=t,Right=Math.Max(l,Math.Min(a.Right,b.Right)),Bottom=Math.Max(t,Math.Min(a.Bottom,b.Bottom)) };
    }
    public static StatusBarNative.Rect ClientBounds(IntPtr window,IntPtr main,uint pid) {
        ProxyUiSmoke.CheckOwner(window,pid); ProxyUiSmoke.CheckOwner(main,pid);
        var old=StatusBarNative.EnterDpi();
        try {
            var client=StatusBarNative.Client(window,pid); var p=new StatusBarNative.Point();
            if(!ClientToScreen(window,ref p) || !ScreenToClient(main,ref p)) throw new Exception("Client origin mapping failed");
            return new StatusBarNative.Rect { Left=p.X,Top=p.Y,Right=p.X+client.Width,Bottom=p.Y+client.Height };
        } finally { StatusBarNative.RestoreDpi(old); }
    }
    public static StatusBarNative.Rect Document(StatusBarNative.Rect bounds,int offset,uint dpi) {
        int dy=offset+Scale(44,dpi);
        return new StatusBarNative.Rect { Left=bounds.Left,Top=bounds.Top+dy,Right=bounds.Right,Bottom=bounds.Bottom+dy };
    }
    // fullControl is the complete HWND rect in HOST-client coordinates. Use
    // the measured client width AFTER scrollbar visibility settles, not the
    // main window/host outer width or an intersection with its clipping area.
    public static RightMarginProof PageRightMargin(int hostClientWidth,StatusBarNative.Rect fullControl,uint dpi) {
        if(hostClientWidth<1 || dpi==0) throw new ArgumentException("Invalid host-client width/DPI");
        int margin=Scale(24,dpi);
        var proof=new RightMarginProof { HostClientWidth=hostClientWidth,Right=fullControl.Right,
            ExpectedRight=hostClientWidth-margin,Margin=hostClientWidth-fullControl.Right,ExpectedMargin=margin };
        if(fullControl.Width<=0 || fullControl.Height<=0)
            proof.Failure="Missing complete stretched-control rectangle";
        else if(Math.Abs((long)proof.Right-proof.ExpectedRight)>1)
            proof.Failure="Right margin must be 24 DIP +/-1 physical pixel relative to actual host client: right="+proof.Right+
                ", expected="+proof.ExpectedRight+", hostClientWidth="+hostClientWidth+", margin="+proof.Margin;
        return proof;
    }
    static void ValidatePageRightMargins() {
        foreach(uint dpi in new uint[]{96,120,144,192}) {
            foreach(int width in new int[]{744,983,1000}) {
                foreach(int scrollbar in new int[]{0,17}) {
                    int hostWidth=Scale(width,dpi)-Scale(scrollbar,dpi);
                    var full=new StatusBarNative.Rect { Left=Scale(24,dpi),Top=-Scale(60,dpi),
                        Right=hostWidth-Scale(24,dpi),Bottom=Scale(80,dpi) };
                    foreach(int pixel in new int[]{-1,0,1}) {
                        var rounded=full; rounded.Right+=pixel;
                        if(PageRightMargin(hostWidth,rounded,dpi).Failure!=null)
                            throw new Exception("Actual-client right margin +/-1px tolerance rejected");
                    }
                    // Both directions matter: showing the bar can leave a too-wide
                    // control, hiding it can leave unused stale client width.
                    foreach(int drift in new int[]{-Scale(17,dpi),-2,2,Scale(17,dpi)}) {
                        var stale=full; stale.Right+=drift;
                        if(PageRightMargin(hostWidth,stale,dpi).Failure==null)
                            throw new Exception("Stale scrollbar-dependent width/right margin was accepted");
                    }
                }
            }
        }
        var regression=PageRightMargin(983,new StatusBarNative.Rect { Left=24,Top=60,Right=976,Bottom=94 },96);
        if(regression.Failure==null || regression.ExpectedRight!=959)
            throw new Exception("976-vs-959 scrollbar-width regression was not detected");
    }
    // Only these two settings tracks are capped; PageRightMargin remains strict
    // for every stretched input/tail, including after native scrollbar show/hide.
    public static SegmentWidthProof SettingsSegmentWidth(int id,int hostClientWidth,StatusBarNative.Rect fullControl,uint dpi) {
        if(hostClientWidth<1 || dpi==0) throw new ArgumentException("Invalid host-client width/DPI");
        int leftDip,maxDip;
        if(id==2039) { leftDip=24; maxDip=864; }
        else if(id==2030) { leftDip=180; maxDip=456; }
        else throw new ArgumentException("Only settings segments 2039/2030 have bounded widths");
        int left=Scale(leftDip,dpi),margin=Scale(24,dpi);
        int available=Math.Max(1,Math.Max(1,hostClientWidth-2*margin)-Scale(leftDip-24,dpi));
        int width=Math.Min(available,Scale(maxDip,dpi));
        var proof=new SegmentWidthProof { ID=id,HostClientWidth=hostClientWidth,Left=fullControl.Left,ExpectedLeft=left,
            Width=fullControl.Width,ExpectedWidth=width,Right=fullControl.Right,ExpectedRight=left+width,MaxWidthDIP=maxDip };
        if(fullControl.Width<=0 || fullControl.Height<=0)
            proof.Failure="Missing complete settings-segment rectangle";
        else if(Math.Abs((long)proof.Left-proof.ExpectedLeft)>1 || Math.Abs((long)proof.Width-proof.ExpectedWidth)>1 ||
            Math.Abs((long)proof.Right-proof.ExpectedRight)>1)
            proof.Failure="Settings segment must retain its left anchor and exact min(available, DPI cap) width +/-1px: ID="+id+
                ", left="+proof.Left+", expectedLeft="+proof.ExpectedLeft+", width="+proof.Width+", expectedWidth="+proof.ExpectedWidth+
                ", right="+proof.Right+", expectedRight="+proof.ExpectedRight+", hostClientWidth="+hostClientWidth;
        return proof;
    }
    static void ValidateSettingsSegmentWidths() {
        foreach(uint dpi in new uint[]{96,120,144,192}) {
            foreach(int id in new int[]{2039,2030}) {
                int left=Scale(id==2039?24:180,dpi),cap=Scale(id==2039?864:456,dpi),margin=Scale(24,dpi);
                foreach(int width in new int[]{624,659,660,661,744,911,912,913,983,1000,1488,744}) {
                    foreach(int scrollbar in new int[]{0,17}) {
                        int hostWidth=Scale(width,dpi)-Scale(scrollbar,dpi);
                        var full=new StatusBarNative.Rect { Left=left,Top=-Scale(60,dpi),
                            Right=Math.Min(hostWidth-margin,left+cap),Bottom=Scale(80,dpi) };
                        var valid=SettingsSegmentWidth(id,hostWidth,full,dpi);
                        if(valid.Failure!=null || valid.ExpectedRight!=full.Right)
                            throw new Exception("Exact anchored settings segment width rejected");
                        foreach(int pixel in new int[]{-1,0,1}) {
                            var rounded=full; rounded.Right+=pixel;
                            if(SettingsSegmentWidth(id,hostWidth,rounded,dpi).Failure!=null)
                                throw new Exception("Settings segment +/-1px tolerance rejected");
                        }
                        foreach(int drift in new int[]{-Scale(17,dpi),-2,2,Scale(17,dpi)}) {
                            var stale=full; stale.Right+=drift;
                            if(SettingsSegmentWidth(id,hostWidth,stale,dpi).Failure==null)
                                throw new Exception("Stale/overwide/underwide settings segment was accepted");
                            var shifted=full; shifted.Left+=drift; shifted.Right+=drift;
                            if(SettingsSegmentWidth(id,hostWidth,shifted,dpi).Failure==null)
                                throw new Exception("Centered/shifted settings segment was accepted");
                        }
                        // Native scrollbar transitions can cross the cap threshold.
                        foreach(int delta in new int[]{-Scale(17,dpi),Scale(17,dpi)}) {
                            var stale=full; stale.Right=Math.Min(hostWidth+delta-margin,left+cap);
                            if(Math.Abs((long)stale.Right-full.Right)>1 && SettingsSegmentWidth(id,hostWidth,stale,dpi).Failure==null)
                                throw new Exception("Previous-client settings segment width was accepted");
                        }
                        var uncapped=full; uncapped.Right=hostWidth-margin;
                        if(uncapped.Right-full.Right>1 && SettingsSegmentWidth(id,hostWidth,uncapped,dpi).Failure==null)
                            throw new Exception("Uncapped settings segment was accepted");
                    }
                }
            }
        }
        var category=SettingsSegmentWidth(2039,983,new StatusBarNative.Rect { Left=24,Top=60,Right=888,Bottom=94 },96);
        var theme=SettingsSegmentWidth(2030,983,new StatusBarNative.Rect { Left=180,Top=118,Right=636,Bottom=150 },96);
        if(category.Failure!=null || category.ExpectedWidth!=864 || theme.Failure!=null || theme.ExpectedWidth!=456)
            throw new Exception("864/456-DIP settings cap regression");
        bool rejected=false;
        try { SettingsSegmentWidth(2010,983,new StatusBarNative.Rect(),96); }
        catch(ArgumentException) { rejected=true; }
        if(!rejected) throw new Exception("Ordinary settings input acquired a segment-width exemption");
    }
    // Pure geometry: fullTail is the complete HWND rectangle in main-client
    // coordinates, NEVER an intersection with the viewport. Tall editors may
    // extend above the viewport at SB_BOTTOM, but their actual bottom must fit.
    public static TailProof PageTail(int page,int clientHeight,StatusBarNative.Rect clip,
        StatusBarNative.Rect footer,ScrollInfo scroll,StatusBarNative.Rect fullTail,int lastDocumentBottom,uint dpi) {
        if(page<0 || page>3 || dpi==0 || clientHeight<1 || clip.Height<1) throw new ArgumentException("Invalid page-tail geometry");
        int padding=Scale(16,dpi),extent=scroll.Max-scroll.Min+1;
        int expected=page==3?Math.Max(clip.Height,lastDocumentBottom-Scale(44,dpi)+padding):
            Math.Max(clientHeight,Scale(page==2?952:820,dpi))-Scale(44+40,dpi);
        var proof=new TailProof { AtBottom=scroll.Position==scroll.Limit,
            StrictGap=scroll.Position==scroll.Limit && (page!=3 || scroll.Limit>0),
            Gap=footer.Top-fullTail.Bottom,Padding=padding,Extent=extent,ExpectedExtent=expected,LastDocumentBottom=lastDocumentBottom };
        if(scroll.Min!=0 || scroll.Position<scroll.Min || scroll.Position>scroll.Limit || scroll.Page!=(uint)clip.Height)
            proof.Failure="Native scroll range/page does not match the viewport";
        else if(Math.Abs((long)extent-expected)>1)
            proof.Failure="Unexpected/phantom scroll extent: observed="+extent+", expected="+expected;
        else if(fullTail.Width<=0 || fullTail.Height<=0 || Math.Abs((long)fullTail.Bottom+scroll.Position-lastDocumentBottom)>1)
            proof.Failure="Missing complete tail rectangle or inconsistent document/main coordinates";
        else if(proof.AtBottom && (fullTail.Left<clip.Left || fullTail.Right>clip.Right ||
            fullTail.Bottom<=clip.Top || fullTail.Top>=clip.Bottom || fullTail.Bottom>clip.Bottom+1))
            proof.Failure="Actual tail bottom is clipped/outside the viewport; clipped rectangles cannot prove padding";
        else if(proof.StrictGap && Math.Abs((long)proof.Gap-padding)>1)
            proof.Failure="Tail/footer gap must be 16 DIP +/-1 physical pixel: actual="+proof.Gap+", expected="+padding;
        else if(page==3 && scroll.Limit==0 && (long)fullTail.Bottom+padding>clip.Bottom+1)
            proof.Failure="Short settings form plus 16 DIP padding does not fit the viewport";
        return proof;
    }
    static void ValidatePageTails() {
        foreach(uint dpi in new uint[]{96,120,144,192}) {
            foreach(int page in new int[]{0,1,2}) {
                int minimum=page==2?952:820;
                foreach(int height in new int[]{560,680,minimum-28,minimum-1,minimum,1000}) {
                    int client=Scale(height,dpi),virtualHeight=Math.Max(client,Scale(minimum,dpi));
                    var clip=new StatusBarNative.Rect { Top=Scale(44,dpi),Right=Scale(744,dpi),Bottom=client-Scale(40,dpi) };
                    var footer=new StatusBarNative.Rect { Top=clip.Bottom,Right=clip.Right,Bottom=client };
                    var scroll=new ScrollInfo { Min=0,Max=virtualHeight-Scale(84,dpi)-1,Page=(uint)clip.Height };
                    scroll.Position=scroll.Limit;
                    int documentBottom=virtualHeight-Scale(40+16,dpi);
                    var tail=new StatusBarNative.Rect { Left=Scale(24,dpi),Top=Scale(110,dpi)-scroll.Position,
                        Right=clip.Right-Scale(24,dpi),Bottom=documentBottom-scroll.Position };
                    var proof=PageTail(page,client,clip,footer,scroll,tail,documentBottom,dpi);
                    if(proof.Failure!=null || !proof.AtBottom || !proof.StrictGap) throw new Exception("Bottom-padding self-check failed: "+proof.Failure);
                    var legacy=tail; legacy.Bottom-=Scale(28,dpi);
                    if(PageTail(page,client,clip,footer,scroll,legacy,documentBottom-Scale(28,dpi),dpi).Failure==null)
                        throw new Exception("Legacy extra 28-DIP tail space was accepted");
                    var clipped=tail; clipped.Bottom=clip.Bottom+Scale(40,dpi);
                    if(PageTail(page,client,clip,footer,scroll,clipped,clipped.Bottom+scroll.Position,dpi).Failure==null)
                        throw new Exception("Clipped complete tail was accepted");
                    foreach(int pixel in new int[]{-1,1}) {
                        var rounded=tail; rounded.Bottom+=pixel;
                        if(PageTail(page,client,clip,footer,scroll,rounded,documentBottom+pixel,dpi).Failure!=null)
                            throw new Exception("One-physical-pixel tail tolerance rejected");
                    }
                    if(scroll.Limit>0) {
                        tail.Top+=scroll.Position; tail.Bottom+=scroll.Position; scroll.Position=0;
                        proof=PageTail(page,client,clip,footer,scroll,tail,documentBottom,dpi);
                        if(proof.Failure!=null || proof.StrictGap || proof.AtBottom) throw new Exception("Nonterminal scroll must not require a 16-DIP gap");
                    }
                    var oldExtent=scroll; oldExtent.Max-=Scale(28,dpi);
                    if(PageTail(page,client,clip,footer,oldExtent,tail,documentBottom,dpi).Failure==null)
                        throw new Exception("Legacy extent subtraction was accepted");
                }
            }
            foreach(int documentBottomDip in new int[]{326,416,640,464,594,536}) {
                foreach(int height in new int[]{560,1000}) {
                    int client=Scale(height,dpi),documentBottom=Scale(documentBottomDip,dpi);
                    var clip=new StatusBarNative.Rect { Top=Scale(44,dpi),Right=Scale(744,dpi),Bottom=client-Scale(40,dpi) };
                    var footer=new StatusBarNative.Rect { Top=clip.Bottom,Right=clip.Right,Bottom=client };
                    int extent=Math.Max(clip.Height,documentBottom-Scale(44,dpi)+Scale(16,dpi));
                    var scroll=new ScrollInfo { Min=0,Max=extent-1,Page=(uint)clip.Height };
                    scroll.Position=scroll.Limit;
                    var tail=new StatusBarNative.Rect { Left=Scale(24,dpi),Top=documentBottom-Scale(32,dpi)-scroll.Position,
                        Right=Scale(172,dpi),Bottom=documentBottom-scroll.Position };
                    var proof=PageTail(3,client,clip,footer,scroll,tail,documentBottom,dpi);
                    if(proof.Failure!=null || proof.StrictGap!=(scroll.Limit>0)) throw new Exception("Content-sized settings tail self-check failed: "+proof.Failure);
                    var phantom=scroll; phantom.Max+=Scale(28,dpi); phantom.Position=0;
                    tail.Top+=scroll.Position; tail.Bottom+=scroll.Position;
                    if(PageTail(3,client,clip,footer,phantom,tail,documentBottom,dpi).Failure==null)
                        throw new Exception("Phantom settings scroll extent was accepted");
                }
            }
        }
    }
    public static StatusBarNative.Rect Resize(IntPtr window,uint pid,int widthDip,int heightDip) {
        ProxyUiSmoke.CheckOwner(window,pid); var old=StatusBarNative.EnterDpi();
        try {
            var monitor=new MonitorInfo { Size=(uint)Marshal.SizeOf(typeof(MonitorInfo)) };
            if(!GetMonitorInfoW(MonitorFromWindow(window,2),ref monitor)) throw new Exception("Monitor work area unavailable");
            uint dpi=ClarityNative.GetDpiForWindow(window);
            if(dpi==0) throw new Exception("Window DPI unavailable");
            int w=Math.Min(monitor.Work.Width,Scale(widthDip,dpi)),h=Math.Min(monitor.Work.Height,Scale(heightDip,dpi));
            if(!SetWindowPos(window,IntPtr.Zero,monitor.Work.Left,monitor.Work.Top,w,h,0x4014)) throw new Exception("Async outer-DIP resize failed");
            return new StatusBarNative.Rect { Left=monitor.Work.Left,Top=monitor.Work.Top,Right=monitor.Work.Left+w,Bottom=monitor.Work.Top+h };
        } finally { StatusBarNative.RestoreDpi(old); }
    }
    public static void NotifyCombo(IntPtr combo,uint pid) {
        ProxyUiSmoke.CheckOwner(combo,pid);
        IntPtr parent=StatusBarNative.GetParent(combo); ProxyUiSmoke.CheckOwner(parent,pid);
        uint id=(uint)ClarityNative.GetDlgCtrlID(combo);
        if(!PostMessageW(parent,0x111,new UIntPtr(id|0x10000),combo)) throw new Exception("CBN_SELCHANGE post to actual parent failed");
    }
    public static EditProof Edit(IntPtr edit,uint pid) {
        ProxyUiSmoke.CheckOwner(edit,pid);
        if(ProxyUiSmoke.Class(edit)!="Edit") throw new Exception("Expected native Edit");
        IntPtr region=CreateRectRgn(0,0,0,0);
        if(region==IntPtr.Zero) throw new Exception("Local Edit region probe allocation failed");
        try {
            if(GetWindowRgn(edit,region)!=0) throw new Exception("Edit must retain its native rectangular border without a clipping region");
        } finally { DeleteObject(region); }
        long style=unchecked((uint)ProxyUiSmoke.GetWindowLongW(edit,-16)); bool password=(style&0x20)!=0;
        return new EditProof { Handle=edit,Password=password,NativeStyle=style&0xFFFFFF,
            Length=Message(edit,pid,14,0,0),Selection=Message(edit,pid,0xB0,0,0),Margins=Message(edit,pid,0xD4,0,0),
            Mask=Message(edit,pid,0xD2,0,0),Limit=Message(edit,pid,0xD5,0,0),Text=password?null:ClarityNative.Text(edit,pid) };
    }
    static double Linear(int channel) {
        double c=channel/255.0; return c<=0.04045?c/12.92:Math.Pow((c+0.055)/1.055,2.4);
    }
    static double Luminance(Color c) { return 0.2126*Linear(c.R)+0.7152*Linear(c.G)+0.0722*Linear(c.B); }
    static double Contrast(Color a,Color b) {
        double x=Luminance(a),y=Luminance(b); return (Math.Max(x,y)+0.05)/(Math.Min(x,y)+0.05);
    }
    static string Rgb(Color c) { return String.Format("{0},{1},{2}",c.R,c.G,c.B); }
    static Color ColorRef(long value) { return Color.FromArgb((int)(value&255),(int)((value>>8)&255),(int)((value>>16)&255)); }
    public static ContrastProof TableContrast(IntPtr table,uint pid) {
        ProxyUiSmoke.CheckOwner(table,pid);
        if(ProxyUiSmoke.Class(table)!="SysListView32") throw new Exception("Expected native table");
        long fg=Message(table,pid,0x1023,0,0),bg=Message(table,pid,0x1025,0,0);
        if(unchecked((uint)bg)==0xFFFFFFFF) bg=Message(table,pid,0x1000,0,0);
        if(unchecked((uint)fg)>0xFFFFFF || unchecked((uint)bg)>0xFFFFFF) throw new Exception("Native table has unresolved text/background color; readability NOT validated");
        Color text=ColorRef(fg),surface=ColorRef(bg);
        return new ContrastProof { ForegroundRgb=Rgb(text),BackgroundRgb=Rgb(surface),BackgroundLuminance=Luminance(surface),Contrast=Contrast(text,surface),Scope="Actual LVM_GETTEXTCOLOR / GETTEXTBKCOLOR (or GETBKCOLOR); normal unselected text only" };
    }
    public static ContrastProof InspectContrast(string path,StatusBarNative.Rect region,int inset,double minimum) {
        return InspectContrast(path,region,inset,minimum,2);
    }
    public static ContrastProof InspectContrast(string path,StatusBarNative.Rect region,int inset,double minimum,int step) {
        using(var bitmap=new Bitmap(path)) { return InspectContrast(bitmap,region,inset,minimum,step); }
    }
    static ContrastProof InspectContrast(Bitmap bitmap,StatusBarNative.Rect region,int inset,double minimum,int step) {
        // Interior sampling deliberately excludes borders. One-pixel chevrons
        // require dense sampling: a two-pixel grid can miss BOTH diagonal strokes.
        // This remains contrast evidence, not OCR/every-glyph/screen acceptance.
        if(step<1 || step>2 || inset<0) throw new Exception("Invalid contrast sampling step/inset");
            if(region.Width==0 && region.Height==0) region=new StatusBarNative.Rect { Right=bitmap.Width,Bottom=bitmap.Height };
            if(region.Left<0 || region.Top<0 || region.Right>bitmap.Width || region.Bottom>bitmap.Height) throw new Exception("Contrast region outside offscreen PNG");
            region.Left+=inset; region.Top+=inset; region.Right-=inset; region.Bottom-=inset;
            if(region.Width<2 || region.Height<2) throw new Exception("Contrast region too small after border exclusion");
            var counts=new Dictionary<int,int>(); int samples=0,mode=0,peak=0;
            for(int y=region.Top;y<region.Bottom;y+=step) for(int x=region.Left;x<region.Right;x+=step) {
                int key=bitmap.GetPixel(x,y).ToArgb(),n; counts.TryGetValue(key,out n); counts[key]=++n; samples++;
                if(n>peak) { peak=n; mode=key; }
            }
            Color background=Color.FromArgb(mode); double max=1; int ink=0;
            foreach(var pair in counts) { double ratio=Contrast(background,Color.FromArgb(pair.Key)); max=Math.Max(max,ratio); if(ratio>=minimum) ink+=pair.Value; }
            return new ContrastProof { BackgroundRgb=Rgb(background),BackgroundLuminance=Luminance(background),Contrast=max,Samples=samples,InkSamples=ink,SampleStep=step,
                Scope="PrintWindow OFFSCREEN interior color-contrast evidence only; no OCR/glyph/physical-screen readability acceptance" };
    }
    public static string Validate() {
        ValidatePageTails();
        ValidatePageRightMargins();
        ValidateSettingsSegmentWidths();
        if(Marshal.SizeOf(typeof(ScrollInfo))!=28) throw new Exception("Unexpected x64 SCROLLINFO layout");
        var scroll=new ScrollInfo { Min=0,Max=715,Page=400 };
        if(scroll.Limit!=316) throw new Exception("Inclusive scroll range self-check failed");
        var a=new StatusBarNative.Rect { Right=10,Bottom=10 };
        if(Intersect(a,new StatusBarNative.Rect { Top=10,Right=10,Bottom=20 }).Height!=0 ||
           Intersect(a,new StatusBarNative.Rect { Top=20,Right=10,Bottom=30 }).Height!=0) throw new Exception("Clip edge/disjoint self-check failed");
        var document=Document(new StatusBarNative.Rect { Top=-20,Bottom=10 },36,144);
        if(document.Top!=82 || document.Bottom!=112 || Math.Abs(Contrast(Color.Black,Color.White)-21)>0.001) throw new Exception("Document coordinate/contrast math self-check failed");
        var empty=new StatusBarNative.Rect(); var nonempty=new StatusBarNative.Rect { Right=1,Bottom=1 };
        if(!MetadataShape(2107,"Static",0,empty,empty) || MetadataShape(2108,"Static",0,empty,empty) ||
           MetadataShape(2107,"Edit",0,empty,empty) || MetadataShape(2107,"Static",0x10000,empty,empty) ||
           MetadataShape(2107,"Static",0x100,empty,empty) || MetadataShape(2107,"Static",0,nonempty,empty))
            throw new Exception("Metadata-only exemption must reject other IDs/classes, TabStop/notify or nonzero area");
        var arrow=ComboArrowRegion(new StatusBarNative.Rect { Right=173,Bottom=28 },96);
        if(arrow.Left!=143 || arrow.Top!=1 || arrow.Right!=172 || arrow.Bottom!=27) throw new Exception("Custom padded ComboBox arrow-region self-check failed");
        using(var bitmap=new Bitmap(173,28)) {
            using(var g=Graphics.FromImage(bitmap)) { g.Clear(Color.Black); }
            int cx=(arrow.Left+arrow.Right)/2,cy=(arrow.Top+arrow.Bottom)/2,radius=4;
            for(int x=cx-radius;x<=cx+radius;x++) bitmap.SetPixel(x,cy+radius/2-Math.Abs(x-cx),Color.White);
            var sparse=InspectContrast(bitmap,arrow,3,3.0,2); var dense=InspectContrast(bitmap,arrow,3,3.0,1);
            if(sparse.InkSamples!=0 || dense.InkSamples<2 || dense.Contrast<3.0) throw new Exception("One-pixel chevron aliasing/dense contrast self-check failed");
        }
        return "SCROLLINFO, full-tail 16-DIP padding/terminal-only checks, content-sized settings/phantom extents, actual-host-client 24-DIP right margins (+/-1px, scrollbar show/hide and 976-vs-959 regressions), exact anchored 864/456-DIP settings segment widths (narrow/threshold/wide, stale client/cap/anchor rejection), 96/120/144/192-DPI geometry, clip/document bounds, narrow metadata contract, padded arrow geometry and dense one-pixel chevron contrast";
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
                if(r.Width<1 || r.Height<1 || r.Width>8192 || r.Height>8192) throw new Exception("Invalid physical-pixel PrintWindow OFFSCREEN size");
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
                    // Descriptive statistics only, not fixed legacy-palette or
                    // per-item blue-border acceptance thresholds.
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
                    if(after.Width!=r.Width || after.Height!=r.Height) throw new Exception("Window resized during PrintWindow OFFSCREEN rendering: "+path);
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
$tokens=$null; $parseErrors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($PSCommandPath,[ref]$tokens,[ref]$parseErrors)
if ($parseErrors.Count -gt 0) { throw 'PowerShell parser errors' }
foreach ($call in $ast.FindAll({ param($n) $n -is [Management.Automation.Language.InvokeMemberExpressionAst] -and $n.Static -and $n.Expression.Extent.Text -eq '[ModernNative]' },$true)) {
    if (@([ModernNative].GetMethods() | Where-Object { $_.IsStatic -and $_.Name -eq $call.Member.Value }).Count -eq 0) { throw "Missing public ModernNative helper $($call.Member.Value)" }
}
$helperValidation=[ModernNative]::Validate()
if ($ValidateOnly) {
    Write-Output "PASS: PowerShell parser/public helper call sites, declaration extraction, combined C# compilation and pure helper self-checks: $helperValidation"
    Write-Output 'VALIDATION ONLY: no EXE lookup/build, GUI, fixture, mock, clipboard, Startup, network or acceptance tests run'
    return
}

$session = [Diagnostics.Process]::GetCurrentProcess().SessionId
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$mutexName = "Local\AIRelay.$sid.$session.v1"
$child=$null; $childHandle=[IntPtr]::Zero; $childStart=$null; $ownedPid=0
$window=[IntPtr]::Zero; $hostWindow=[IntPtr]::Zero; $footer=[IntPtr]::Zero
$mock=$null; $temp=$null; $failure=$null
$checks=[Collections.Generic.List[object]]::new()
$shots=[Collections.Generic.List[object]]::new()
$matrix=[Collections.Generic.List[object]]::new()
$settingsMatrix=[Collections.Generic.List[object]]::new()
$historyTailMatrix=[Collections.Generic.List[object]]::new()
$readability=[Collections.Generic.List[object]]::new()
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
    $others=@(Get-Process -Name 'ai-relay*' -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $ownedPid })
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
function Fence {
    $before=[regex]::Match((Text 2354),'(?:^|;)fence=(\d+)(?:;|$)')
    if (-not $before.Success) { throw 'Missing history owner-thread fence metadata 2354' }
    $sequence=[uint64]$before.Groups[1].Value
    [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,2397)
    Wait-State 'owner-thread queue fence' {
        $now=[regex]::Match((Text 2354),'(?:^|;)fence=(\d+)(?:;|$)')
        $now.Success -and [uint64]$now.Groups[1].Value -gt $sequence
    }
    if ((Text 2354) -notmatch '(?:^|;)rejected=0(?:;|$)') { throw 'History rejected a command; no successful interaction claim' }
    if ([ProxyUiSmoke]::Dialog([uint32]$ownedPid) -ne [IntPtr]::Zero) { throw 'Unexpected owned modal dialog; no automatic confirmation sent' }
}
function Scroll([int]$Action) {
    Message 2400 0x115 ([uint32]$Action) | Out-Null
    Fence
}
function Ensure-Reachable([IntPtr]$Handle) {
    Assert-Child; Fence
    if (-not [ProxyUiSmoke]::IsWindowVisible($Handle)) { throw 'Refusing input to hidden control' }
    if ([StatusBarNative]::GetParent($Handle) -ne $hostWindow) { return }
    for ($attempt=0;$attempt -lt 32;$attempt++) {
        $r=[ModernNative]::Bounds($Handle,$hostWindow,[uint32]$ownedPid)
        $client=[StatusBarNative]::Client($hostWindow,[uint32]$ownedPid)
        if ($r.Left -lt 0 -or $r.Right -gt $client.Width -or $r.Height -gt $client.Height) { throw "Control cannot fully fit viewport before input: $r client=$client" }
        if ($r.Top -ge 0 -and $r.Bottom -le $client.Height) { return }
        $old=[ModernNative]::Scroll($hostWindow,[uint32]$ownedPid).Position
        if ($r.Top -lt 0) { Scroll 0 } else { Scroll 1 }
        if ([ModernNative]::Scroll($hostWindow,[uint32]$ownedPid).Position -eq $old) { break }
    }
    throw "Control did not become reachable through bounded native scrolling: ID=$([ClarityNative]::GetDlgCtrlID($Handle))"
}
function Click([int]$Id) {
    if ($Id -notin @(2102,2103,2317,2318,2320,2228)) { throw "Action not allowlisted by modern smoke: $Id" }
    $h=Control $Id; Ensure-Reachable $h
    [ModernNative]::Click($h,[uint32]$ownedPid)
    Fence
}
function Select-Segment([int]$Id,[int]$Index,[switch]$MainTab) {
    $tab=Control $Id
    if ([ProxyUiSmoke]::Class($tab) -ne 'SysTabControl32') { throw "ID $Id is not native segmented SysTabControl32" }
    Ensure-Reachable $tab
    [ModernNative]::ClickTab($childHandle,[uint32]$ownedPid,$tab,$Index)
    Fence
    Wait-State "native Tab $Id selection $Index" { (Message $Id 0x130B) -eq $Index }
    if (-not $MainTab -and (Message $Id 0x147) -ne $Index) { throw "ID $Id CB_GETCURSEL differs from TCM_GETCURSEL" }
}
function Select-Page([int]$Index,[int]$Expected) {
    Select-Segment 2100 $Index -MainTab
    Wait-State "page $Index control $Expected" { Visible $Expected }
}
function Select-Category([int]$Index) {
    Select-Page 3 2039
    Scroll 6
    Select-Segment 2039 $Index
    $expected=@(2001,2040,2043,2050,2010,2030)[$Index]
    Wait-State "six-item settings segment $Index content" { Visible $expected }
}
function Select-Combo([int]$Id,[int]$Index) {
    $combo=Control $Id
    if ([ProxyUiSmoke]::Class($combo) -ne 'ComboBox') { throw "ID $Id expected conventional ComboBox (not a segmented choice)" }
    Ensure-Reachable $combo
    if ((Message $Id 0x14E ([uint32]$Index)) -ne $Index) { throw "Combo $Id CB_SETCURSEL failed" }
    [ModernNative]::NotifyCombo($combo,[uint32]$ownedPid)
    Fence
}
function Resize-Window([int]$Width,[int]$Height) {
    Assert-Child
    $wanted=[ModernNative]::Resize($window,[uint32]$ownedPid,$Width,$Height)
    Wait-State "outer ${Width}x${Height} DIP/work-area clamp" {
        $actual=[StatusBarNative]::Outer($window,[uint32]$ownedPid)
        $actual.Width -eq $wanted.Width -and $actual.Height -eq $wanted.Height
    }
    Fence
    $dpi=[ClarityNative]::GetDpiForWindow($window)
    $clamped=$wanted.Width -ne [ModernNative]::Scale($Width,$dpi) -or $wanted.Height -ne [ModernNative]::Scale($Height,$dpi)
    Record 'outer DIP request and actual physical-pixel dimensions' @{ RequestedDIP="${Width}x${Height}"; Actual="$wanted"; DPI=$dpi; WorkAreaClamped=$clamped; InteractiveMinTrack='NOT tested' }
    return $wanted
}
function Snapshot([string]$Name,[IntPtr]$Handle,[switch]$Outer,[object]$Region=$null) {
    Assert-Child
    if (-not [ProxyUiSmoke]::IsWindowVisible($Handle)) { throw "Screenshot target hidden: $Name" }
    $rect=New-Object StatusBarNative+Rect
    if ($null -ne $Region) { $rect=$Region }
    $shot=[ModernNative]::Capture($Handle,[uint32]$ownedPid,(Join-Path $output "$Name-PRINTWINDOW-OFFSCREEN.png"),(-not $Outer),$rect)
    $scope=if ($Handle -eq $window) { 'Whole-window rendering with viewport clipping' } else { 'Isolated HWND rendering; may include content currently clipped by its parent' }
    $shots.Add([pscustomobject]@{ Name=$Name; CaptureKind='PrintWindow OFFSCREEN, NOT desktop capture'; Scope=$scope; DPI=[ClarityNative]::GetDpiForWindow($Handle); Shot=$shot })
    Record "PrintWindow OFFSCREEN $Name" $shot
    return $shot
}
function Capture-Segment([int]$Id,[int]$Index,[string]$Prefix='dark') {
    Snapshot "$Prefix-segment-$Id-$Index" (Control $Id) | Out-Null
}
function Assert-RenderedContrast([object]$Shot,[string]$Name,[object]$Region=$null,[double]$Minimum=3.0,[switch]$Dense) {
    $rect=New-Object StatusBarNative+Rect
    if ($null -ne $Region) { $rect=$Region }
    $inset=[ModernNative]::Scale(3,[ClarityNative]::GetDpiForWindow($window))
    $sampleStep=if ($Dense) { 1 } else { 2 }
    $proof=[ModernNative]::InspectContrast($Shot.Path,$rect,$inset,$Minimum,$sampleStep)
    $accepted=$proof.InkSamples -ge 2 -and $proof.Contrast -ge $Minimum
    $readability.Add([pscustomobject]@{ Name=$Name; PNG=$Shot.Path; MinimumContrast=$Minimum; Accepted=$accepted; Proof=$proof; Coverage='Interior contrast evidence, NOT every glyph or full visual acceptance' })
    Record "$Name offscreen interior contrast (no legacy RGB/border requirement)" $proof
    if (-not $accepted) {
        $problem="Insufficient interior foreground/background contrast evidence: $Name; inspect $($Shot.Path); readability NOT validated"
        $uiFailures.Add($problem); Write-Warning $problem
    }
    return $proof
}
function Assert-TableContrast([string]$Name) {
    $proof=[ModernNative]::TableContrast((Control 2337),[uint32]$ownedPid)
    $readability.Add([pscustomobject]@{ Name=$Name; MinimumContrast=4.5; Accepted=($proof.Contrast -ge 4.5); Proof=$proof; Coverage='Native normal/unselected table text colors only' })
    Record "$Name native table text/background contrast" $proof
    if ($proof.Contrast -lt 4.5) {
        $problem="Native table normal text contrast below 4.5:1: $Name; $($proof.Contrast)"
        $uiFailures.Add($problem); Write-Warning $problem
    }
    return $proof
}
function Assert-ThemePair([object]$Light,[object]$Dark,[string]$Name) {
    # Theme polarity and a broad relative difference, not exact Palette RGBs,
    # absolute mean-luminance/white-tile limits or a border around every item.
    $accepted=$Light.BackgroundLuminance - $Dark.BackgroundLuminance -ge 0.20
    if (-not $accepted) {
        $problem="Light/dark interior backgrounds are not distinctly ordered: $Name"
        $uiFailures.Add($problem); Write-Warning $problem
    }
    Record "$Name light/dark relative surface change" @{ Light=$Light; Dark=$Dark; MinimumLuminanceDifference=0.20; Accepted=$accepted }
}
function Fixed-Footer {
    $client=[StatusBarNative]::Client($window,[uint32]$ownedPid)
    $height=[ModernNative]::Scale(40,[ClarityNative]::GetDpiForWindow($window))
    $found=@([ModernNative]::DirectChildren($window,[uint32]$ownedPid) | Where-Object {
        if ($_ -eq $hostWindow -or [ProxyUiSmoke]::Class($_) -ne 'Static' -or -not [ProxyUiSmoke]::IsWindowVisible($_)) { return $false }
        $r=[StatusBarNative]::InParent($_,$window,[uint32]$ownedPid)
        $r.Left -eq 0 -and $r.Right -eq $client.Width -and $r.Top -eq $client.Height-$height -and $r.Bottom -eq $client.Height
    })
    if ($found.Count -ne 1) { throw 'Cannot uniquely identify fixed 40-DIP status bar by geometry' }
    return $found[0]
}
function Assert-OverviewMetadata([string]$Label) {
    Assert-Child
    $found=@([ModernNative]::DirectChildren($hostWindow,[uint32]$ownedPid) | Where-Object { [ClarityNative]::GetDlgCtrlID($_) -eq 2107 })
    if ($found.Count -ne 1) { throw "$Label expected exactly one legacy snapshot metadata STATIC 2107" }
    $metadata=$found[0]
    Wait-State 'bounded 2107 legacy snapshot text ready' { [ModernNative]::BoundedText($metadata,[uint32]$ownedPid,4096).Contains('本次程序会话累计') }
    $proof=[ModernNative]::OverviewMetadata($window,$hostWindow,$metadata,[uint32]$ownedPid)
    # A 0x0 metadata HWND has no document row: scrolling may leave its origin
    # outside the page. Require NO area/intersection instead of a fake row position.
    $targets=[Collections.Generic.List[IntPtr]]::new()
    $targets.Add($hostWindow); $targets.Add((Control 2100)); $targets.Add((Fixed-Footer))
    foreach ($h in [ModernNative]::DirectChildren($hostWindow,[uint32]$ownedPid)) {
        if ($h -ne $metadata -and [ProxyUiSmoke]::IsWindowVisible($h)) { $targets.Add($h) }
    }
    foreach ($h in $targets) {
        $r=[ModernNative]::Bounds($h,$window,[uint32]$ownedPid)
        $intersection=[ModernNative]::Intersect($proof.Bounds,$r)
        if ($intersection.Width -gt 0 -or $intersection.Height -gt 0) { throw "$Label metadata 2107 occupies/intersects visual content: ID=$([ClarityNative]::GetDlgCtrlID($h))" }
    }
    Record "$Label bounded read-only overview metadata (not a layout row)" @{ Proof=$proof; NonIntersectingTargets=$targets.Count; ScreenArea=0; TextMutated=$false }
}
function Assert-PageLayout([string]$Label,[ValidateSet('any','top','bottom')][string]$Endpoint='any') {
    Assert-Child; Fence
    $startFailures=$uiFailures.Count
    $dpi=[ClarityNative]::GetDpiForWindow($window)
    $client=[StatusBarNative]::Client($window,[uint32]$ownedPid)
    $hostRect=[StatusBarNative]::InParent($hostWindow,$window,[uint32]$ownedPid)
    $hostClient=[StatusBarNative]::Client($hostWindow,[uint32]$ownedPid)
    $clip=[ModernNative]::ClientBounds($hostWindow,$window,[uint32]$ownedPid)
    $tabRect=[StatusBarNative]::InParent((Control 2100),$window,[uint32]$ownedPid)
    $footer=Fixed-Footer
    $footerRect=[StatusBarNative]::InParent($footer,$window,[uint32]$ownedPid)
    $scroll=[ModernNative]::Scroll($hostWindow,[uint32]$ownedPid)
    if ([StatusBarNative]::GetParent($hostWindow) -ne $window -or $hostRect.Left -ne 0 -or $hostRect.Right -ne $client.Width -or
        $hostRect.Top -ne [ModernNative]::Scale(44,$dpi) -or $hostRect.Bottom -ne $footerRect.Top -or $tabRect.Bottom -gt $hostRect.Top -or
        ([ProxyUiSmoke]::GetWindowLongW($hostWindow,-20) -band 0x10000) -eq 0 -or ([ProxyUiSmoke]::GetWindowLongW($hostWindow,-16) -band 0x02000000) -eq 0) {
        $uiFailures.Add("$Label fixed 44-DIP header / 40-DIP footer / clipping-host contract failed")
    }
    if ($scroll.Position -lt $scroll.Min -or $scroll.Position -gt $scroll.Limit -or $scroll.Page -ne $hostClient.Height) { $uiFailures.Add("$Label native scroll range/page mismatch") }
    if (($Endpoint -eq 'top' -and $scroll.Position -ne $scroll.Min) -or ($Endpoint -eq 'bottom' -and $scroll.Position -ne $scroll.Limit)) {
        $uiFailures.Add("$Label requested $Endpoint endpoint not reached: position=$($scroll.Position), limit=$($scroll.Limit)")
    }
    foreach ($h in [ModernNative]::DirectChildren($window,[uint32]$ownedPid)) {
        if ($h -eq $hostWindow -or -not [ProxyUiSmoke]::IsWindowVisible($h)) { continue }
        $r=[ModernNative]::Bounds($h,$window,[uint32]$ownedPid)
        if ($r.Left -lt 0 -or $r.Top -lt 0 -or $r.Right -gt $client.Width -or $r.Bottom -gt $client.Height -or
            ($h -ne $footer -and $r.Bottom -gt $hostRect.Top)) { $uiFailures.Add("$Label fixed main child outside header/footer: ID=$([ClarityNative]::GetDlgCtrlID($h)) rect=$r") }
    }
    $active=Message 2100 0x130B
    Assert-OverviewMetadata $Label
    $anchors=@{}
    switch ($active) {
        0 { $anchors=@{2108=108;2109=282;2102=220;2103=220;2104=220;2105=220;2106=220} }
        1 {
            $anchors=@{2300=60}
            if (Visible 2020) { $anchors[2020]=110 }
            elseif (-not (Visible 2351)) {
                $advanced=Visible 2310
                $anchors[2320]=if ($advanced) { 386 } else { 232 }
                $anchors[2332]=312
                $resultTop=if ($advanced) { 538 } else { 592 }
                foreach ($id in @(2330,2331,2337)) { $anchors[$id]=$resultTop }
            }
        }
        2 { $anchors=@{2200=324;2207=146;2224=208;2225=208;2226=244;2228=244} }
        3 { $anchors=@{2039=60} }
    }
    $rects=[Collections.Generic.List[object]]::new()
    $documentBottom=$scroll.Max+1+[ModernNative]::Scale(44,$dpi)
    foreach ($h in [ModernNative]::DirectChildren($hostWindow,[uint32]$ownedPid)) {
        if (-not [ProxyUiSmoke]::IsWindowVisible($h)) { continue }
        $id=[ClarityNative]::GetDlgCtrlID($h); $kind=[ProxyUiSmoke]::Class($h)
        # Only 2107 has the independently checked, zero-area metadata contract.
        # Every other visible child (including any unexpected 0x0 HWND) is tested.
        if ($id -eq 2107) { continue }
        $raw=[ModernNative]::Bounds($h,$hostWindow,[uint32]$ownedPid)
        $document=[ModernNative]::Document($raw,$scroll.Position,$dpi)
        $inMain=[ModernNative]::Bounds($h,$window,[uint32]$ownedPid)
        $effective=[ModernNative]::Intersect($inMain,$clip)
        $rects.Add([pscustomobject]@{ Handle=$h; ID=$id; Class=$kind; Document=$document; Viewport=$raw; Effective=$effective; Clipped=($effective.Width -lt $raw.Width -or $effective.Height -lt $raw.Height) })
        if ($raw.Width -le 0 -or $raw.Height -le 0 -or $document.Left -lt 0 -or $document.Right -gt $hostClient.Width -or $document.Top -lt 0 -or $document.Bottom -gt $documentBottom) {
            $uiFailures.Add("$Label invalid document bounds ID=$id class=$kind document=$document extentBottom=$documentBottom host=$hostClient")
        }
        if ($anchors.ContainsKey($id) -and [Math]::Abs($document.Top-[ModernNative]::Scale($anchors[$id],$dpi)) -gt 1) { $uiFailures.Add("$Label stale document coordinate ID=$id top=$($document.Top) expectedDIP=$($anchors[$id]) scroll=$($scroll.Position)") }
        if ($effective.Width -gt 0 -and $effective.Height -gt 0) {
            foreach ($fixed in @($tabRect,$footerRect)) {
                $overlap=[ModernNative]::Intersect($effective,$fixed)
                if ($overlap.Width -gt 0 -and $overlap.Height -gt 0) { $uiFailures.Add("$Label effective page content covers fixed chrome ID=$id") }
            }
        }
    }
    if ($rects.Count -eq 0) { $uiFailures.Add("$Label empty visible viewport child set; layout NOT validated") }
    for ($a=0;$a -lt $rects.Count;$a++) {
        for ($b=$a+1;$b -lt $rects.Count;$b++) {
            # Check the complete visible-page document, including currently scrolled
            # out controls. Clipping is intentional, overlapping document rows is not.
            $overlap=[ModernNative]::Intersect($rects[$a].Document,$rects[$b].Document)
            if ($overlap.Width -gt 0 -and $overlap.Height -gt 0) { $uiFailures.Add("$Label document overlap: ID=$($rects[$a].ID) [$($rects[$a].Document)] vs ID=$($rects[$b].ID) [$($rects[$b].Document)]") }
        }
    }
    $tailIds=switch ($active) {
        0 { @(2109) }
        1 {
            if (Visible 2020) { @(2020) }
            elseif (Visible 2351) { @(2325) }
            else { @(2337,2330,2331) }
        }
        2 { @(2221) }
        3 { @(2001) }
        default { throw "$Label unknown active page $active" }
    }
    $tails=@($rects | Where-Object { $_.ID -in $tailIds })
    $rightMarginIds=if ($active -eq 3) { @(2010) } else { @($tailIds) }
    $wideControls=@($rects | Where-Object { $_.ID -in $rightMarginIds })
    if ($active -eq 3) {
        # Settings lays out this uncapped input even in other categories. Keep
        # actual-client scrollbar-width coverage when both tracks reach their caps.
        $wideControls=@([pscustomobject]@{ ID=2010; Handle=(Control 2010) })
    }
    $rightMargins=[Collections.Generic.List[object]]::new()
    if ($wideControls.Count -ne 1) {
        $uiFailures.Add("$Label expected exactly one stretched input/tail for right-margin IDs $($rightMarginIds -join ','); found $($wideControls.Count)")
    }
    foreach ($wide in $wideControls) {
        # Independent of vertical position: assert at TOP/BOTTOM/cache even if
        # an input or a tall result is vertically clipped. Never crop right.
        $full=[StatusBarNative]::InParent($wide.Handle,$hostWindow,[uint32]$ownedPid)
        $margin=[ModernNative]::PageRightMargin($hostClient.Width,$full,$dpi)
        $rightMargins.Add([pscustomobject]@{ ID=$wide.ID; FullHostClientRect=$full; Proof=$margin })
        if ($null -ne $margin.Failure) { $uiFailures.Add("$Label stretched control ID=$($wide.ID): $($margin.Failure)") }
    }
    $segmentWidths=[Collections.Generic.List[object]]::new()
    if ($active -eq 3) {
        $segmentIds=if ((Message 2039 0x147) -eq 5) { @(2039,2030) } else { @(2039) }
        foreach ($id in $segmentIds) {
            $segments=@($rects | Where-Object { $_.ID -eq $id })
            if ($segments.Count -ne 1) { $uiFailures.Add("$Label expected one visible settings segment ID=$id; found $($segments.Count)"); continue }
            $full=[StatusBarNative]::InParent($segments[0].Handle,$hostWindow,[uint32]$ownedPid)
            $width=[ModernNative]::SettingsSegmentWidth($id,$hostClient.Width,$full,$dpi)
            $segmentWidths.Add([pscustomobject]@{ ID=$id; FullHostClientRect=$full; Proof=$width })
            if ($null -ne $width.Failure) { $uiFailures.Add("$Label settings segment ID=${id}: $($width.Failure)") }
        }
    }
    $tailProof=$null; $tailFull=$null
    if ($tails.Count -ne 1) {
        $uiFailures.Add("$Label expected exactly one visible page tail among IDs $($tailIds -join ','); found $($tails.Count)")
    } else {
        $tail=$tails[0]
        if ($active -eq 3) {
            $actions=@($rects | Where-Object { $_.ID -in @(2001,2002,2003,2004) })
            $lastVisibleBottom=($rects | ForEach-Object { $_.Document.Bottom } | Measure-Object -Maximum).Maximum
            if ($actions.Count -ne 4 -or @($actions | Where-Object { [Math]::Abs($_.Document.Bottom-$lastVisibleBottom) -gt 1 }).Count -gt 0) {
                $uiFailures.Add("$Label settings action row must be the current visible children's actual end; no hidden-category tail allowed")
            }
        } else { $lastVisibleBottom=$tail.Document.Bottom }
        # Read the FULL native HWND rect afresh, not Effective/Intersect or the
        # viewport-clipped screenshot. A tall tail may extend above the viewport.
        $tailFull=[StatusBarNative]::InParent($tail.Handle,$window,[uint32]$ownedPid)
        $tailProof=[ModernNative]::PageTail([int]$active,$client.Height,$clip,$footerRect,$scroll,$tailFull,[int]$lastVisibleBottom,$dpi)
        if ($null -ne $tailProof.Failure) { $uiFailures.Add("$Label tail ID=$($tail.ID): $($tailProof.Failure)") }
    }
    Record "$Label viewport/document geometry (native closed combo bounds; complete tail; actual-client right margin; capped settings segments)" @{ Page=$active; Endpoint=$Endpoint; DPI=$dpi; Host="$hostRect"; HostClient="$hostClient"; Clip="$clip"; Footer="$footerRect"; Scroll=$scroll; DocumentBottom=$documentBottom; TailIDs=@($tailIds); FullTail=$tailFull; TailProof=$tailProof; RightMarginIDs=@($rightMarginIds); RightMargins=$rightMargins.ToArray(); SegmentWidths=$segmentWidths.ToArray(); Children=$rects.ToArray() }
    for ($i=$startFailures;$i -lt $uiFailures.Count;$i++) { Write-Warning $uiFailures[$i] }
}
function Assert-HistoryLayout([string]$Label) { Assert-PageLayout $Label }
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
    if (Test-Path -LiteralPath (Join-Path $temp 'config.toml.bak')) { throw 'Unexpected config backup: an implicit Save may have occurred' }
}
function Assert-InputsPreserved([string]$Label) {
    foreach ($old in $editBaseline) {
        Assert-Child
        $now=[ModernNative]::Edit($old.Handle,[uint32]$ownedPid)
        foreach ($key in @('Password','NativeStyle','Length','Text','Selection','Margins','Mask','Limit')) {
            if ($now.$key -cne $old.$key) { throw "$Label changed native Edit $($old.Handle) property $key (password text is never read)" }
        }
    }
}
function Capture-SettingsTailMatrix([string]$ThemeName,[string]$SizeName,[object]$Actual) {
    foreach ($category in 0..5) {
        $label="$ThemeName-$SizeName-settings-category-$category"
        Select-Category $category
        Scroll 6
        Assert-PageLayout "$label-top" -Endpoint top
        $top=Snapshot "$label-top" $window -Outer
        Scroll 7
        Assert-PageLayout "$label-bottom" -Endpoint bottom
        $scroll=[ModernNative]::Scroll($hostWindow,[uint32]$ownedPid)
        $bottom=Snapshot "$label-bottom" $window -Outer
        # A settings cache hit must keep this category's clamped terminal offset;
        # a top/bottom screenshot alone cannot expose stale cached extents.
        Select-Page 0 2109
        Select-Page 3 2039
        if ((Message 2039 0x147) -ne $category -or [ModernNative]::Scroll($hostWindow,[uint32]$ownedPid).Position -ne $scroll.Position) {
            $uiFailures.Add("$label cached return changed category/terminal scroll position")
        }
        Assert-PageLayout "$label-cached-bottom" -Endpoint bottom
        $settingsMatrix.Add([pscustomobject]@{ Theme=$ThemeName; Category=$category; RequestedOuterDIP=$SizeName; ActualOuterPhysical="$Actual";
            DPI=[ClarityNative]::GetDpiForWindow($window); Top=$top.Path; Bottom=$bottom.Path; BottomOffset=$scroll.Position; Limit=$scroll.Limit;
            TerminalGap='16 DIP +/-1px if scrolling; short forms retain natural whitespace with content-sized extent'; CachedReturnChecked=$true })
        Assert-InputsPreserved "$label top/bottom/cache"
        Assert-ConfigUnchanged
    }
    Select-Category 0
}
function Capture-HistoryTailMatrix([string]$ThemeName,[string]$SizeName,[object]$Actual) {
    Select-Page 1 2300
    Wait-State 'history idle before layout-only subview matrix' { (Text 2354).StartsWith('idle=1;') }
    if (Visible 2351) { throw 'History layout-only matrix requires statistics, not an implicit management/storage query' }
    $sequenceBefore=[regex]::Match((Text 2354),'(?:^|;)sequence=(\d+)(?:;|$)').Value
    if ([string]::IsNullOrEmpty($sequenceBefore)) { throw 'Missing history operation sequence before layout-only subview matrix' }
    $views=@(
        @{ Name='session'; Session=1; Table=0; Advanced=$false; Tail=2020 },
        @{ Name='detail'; Session=0; Table=0; Advanced=$false; Tail=2337 },
        @{ Name='groups'; Session=0; Table=1; Advanced=$false; Tail=2337 },
        @{ Name='summary-text'; Session=0; Table=2; Advanced=$false; Tail=2330 },
        @{ Name='metadata'; Session=0; Table=3; Advanced=$false; Tail=2331 },
        @{ Name='advanced-detail'; Session=0; Table=0; Advanced=$true; Tail=2337 }
    )
    foreach ($view in $views) {
        $label="$ThemeName-$SizeName-history-$($view.Name)"
        Select-Segment 2300 $view.Session
        if ($view.Session -eq 0) {
            if ((Visible 2310) -ne $view.Advanced) { Click 2318 }
            Select-Segment 2338 $view.Table
        }
        Wait-State "$label visible complete tail" { Visible $view.Tail }
        Scroll 6
        Assert-PageLayout "$label-top" -Endpoint top
        Scroll 7
        Assert-PageLayout "$label-bottom" -Endpoint bottom
        $scroll=[ModernNative]::Scroll($hostWindow,[uint32]$ownedPid)
        $bottom=Snapshot "$label-bottom" $window -Outer
        Select-Page 0 2109
        Select-Page 1 2300
        if (-not (Visible $view.Tail) -or [ModernNative]::Scroll($hostWindow,[uint32]$ownedPid).Position -ne $scroll.Position) {
            $uiFailures.Add("$label cached return changed tail/terminal scroll position")
        }
        Assert-PageLayout "$label-cached-bottom" -Endpoint bottom
        $historyTailMatrix.Add([pscustomobject]@{ Theme=$ThemeName; View=$view.Name; TailID=$view.Tail; RequestedOuterDIP=$SizeName; ActualOuterPhysical="$Actual";
            DPI=[ClarityNative]::GetDpiForWindow($window); Bottom=$bottom.Path; BottomOffset=$scroll.Position; Limit=$scroll.Limit; CachedReturnChecked=$true })
        Assert-InputsPreserved "$label top/bottom/cache"
        Assert-ConfigUnchanged
    }
    Select-Segment 2300 0
    if (Visible 2310) { Click 2318 }
    Select-Segment 2338 0
    Scroll 7
    Assert-PageLayout "$ThemeName-$SizeName-history-default-restored-bottom" -Endpoint bottom
    $sequenceAfter=[regex]::Match((Text 2354),'(?:^|;)sequence=(\d+)(?:;|$)').Value
    if ($sequenceBefore -cne $sequenceAfter -or -not (Text 2354).StartsWith('idle=1;')) {
        throw 'Layout-only history subview matrix started a backend operation; no no-I/O claim'
    }
    Record "$ThemeName/$SizeName history layout-only subviews; management deliberately not clicked" @{ Views=$views.Count; OperationSequenceUnchanged=$true;
        ManagementTail=2325; ManagementCoverage='Assertion supported, NOT exercised: 2339 automatically starts HistoryStorage disk I/O; no pure UI-only entry exists'; NeverClicked=@(2339,2343,2344,2345,2348,2350,2352) }
}
function Capture-PageMatrix([string]$ThemeName) {
    $sizes=[Collections.Generic.List[string]]::new()
    foreach ($size in @(@(800,680),@(1000,820))) {
        $actual=Resize-Window $size[0] $size[1]
        $sizes.Add("$($actual.Width)x$($actual.Height)")
        $sizeName="$($size[0])x$($size[1])"
        $offsets=@{}
        foreach ($pageIndex in 0..3) {
            $pageName=@('overview','statistics','realtime-logs','settings')[$pageIndex]
            Select-Page $pageIndex (@(2109,2300,2200,2039)[$pageIndex])
            if ($pageIndex -eq 3) { Select-Category 0 }
            Scroll 6
            Assert-PageLayout "$ThemeName-$sizeName-$pageName-top" -Endpoint top
            $top=Snapshot "$ThemeName-$sizeName-$pageName-top" $window -Outer
            $textControl=if ($pageIndex -eq 3) { $draftEdit } else { Control (@(2108,2320,2200)[$pageIndex]) }
            if ([string]::IsNullOrWhiteSpace([ClarityNative]::Text($textControl,[uint32]$ownedPid))) { throw "$pageName text probe empty; readability evidence NOT available" }
            $textShot=Snapshot "$ThemeName-$sizeName-$pageName-text-probe" $textControl
            Assert-RenderedContrast $textShot "$ThemeName/$sizeName/$pageName representative text" -Minimum 4.5 | Out-Null
            Scroll 7
            $offset=[ModernNative]::Scroll($hostWindow,[uint32]$ownedPid).Position
            $offsets[$pageIndex]=$offset
            Assert-PageLayout "$ThemeName-$sizeName-$pageName-bottom" -Endpoint bottom
            $bottom=$null
            if ($offset -gt 0) {
                $bottom=Snapshot "$ThemeName-$sizeName-$pageName-bottom" $window -Outer
            }
            $matrix.Add([pscustomobject]@{ Theme=$ThemeName; Page=$pageName; RequestedOuterDIP=$sizeName; ActualOuterPhysical="$actual"; DPI=[ClarityNative]::GetDpiForWindow($window); CaptureKind='PrintWindow OFFSCREEN, NOT desktop capture'; Top=$top.Path; Bottom=if ($null -eq $bottom) { $null } else { $bottom.Path }; BottomOffset=$offset })
            Assert-InputsPreserved "$ThemeName/$sizeName/$pageName"
            Assert-ConfigUnchanged
        }
        foreach ($pageIndex in 0..3) {
            Select-Page $pageIndex (@(2109,2300,2200,2039)[$pageIndex])
            if ([ModernNative]::Scroll($hostWindow,[uint32]$ownedPid).Position -ne $offsets[$pageIndex]) { $uiFailures.Add("$ThemeName/$sizeName cached page $pageIndex lost its independent scroll offset") }
            Assert-PageLayout "$ThemeName-$sizeName-page$pageIndex-cached-return"
            Assert-InputsPreserved "$ThemeName/$sizeName/cached-page$pageIndex"
        }
        Capture-SettingsTailMatrix $ThemeName $sizeName $actual
        Capture-HistoryTailMatrix $ThemeName $sizeName $actual
    }
    if (@($sizes.ToArray() | Select-Object -Unique).Count -ne 2) { $uiFailures.Add("$ThemeName work-area clamp made both sizes identical; compact/expanded matrix NOT demonstrated") }
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
    Wait-State 'viewport and all history operation controls ready' {
        $h=[ProxyUiSmoke]::GetDlgItem($window,2400)
        $op=[ProxyUiSmoke]::GetDlgItem($window,2354)
        $h -ne [IntPtr]::Zero -and $op -ne [IntPtr]::Zero -and [ClarityNative]::Text($op,[uint32]$ownedPid).StartsWith('idle=1;')
    }
    $hostWindow=Control 2400
    $footer=Fixed-Footer
    Resize-Window 800 680 | Out-Null
    Assert-PageLayout 'initial-compact-overview'
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
    foreach ($id in @(2300,2301,2333,2334,2338,2353,2030,2039)) {
        $h=Control $id
        if ([ProxyUiSmoke]::Class($h) -ne 'SysTabControl32') { throw "ID $id must now be SysTabControl32, actual=$([ProxyUiSmoke]::Class($h))" }
        $old=Message $id 0x147
        $count=Message $id 0x146
        if ($count -ne (Message $id 0x1304) -or ($id -eq 2039 -and $count -ne 6)) { throw "CB_GETCOUNT/TCM_GETITEMCOUNT mismatch (2039 must contain six categories): ID $id" }
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
    Ensure-Reachable $table
    $tableShot=Snapshot 'dark-advanced-table-2337' $table
    Assert-TableContrast 'dark native table 2337' | Out-Null
    Assert-RenderedContrast $tableShot 'dark native table interior' | Out-Null
    $header=[IntPtr](Message 2337 0x101F)
    [ProxyUiSmoke]::CheckOwner($header,[uint32]$ownedPid)
    if ([ProxyUiSmoke]::Class($header) -ne 'SysHeader32') { throw '2337 header is not native SysHeader32' }
    $headerShot=Snapshot 'dark-table-header' $header
    Assert-RenderedContrast $headerShot 'native header interior text' -Minimum 4.5 | Out-Null
    $combo=Control 2310
    $info=[ModernNative]::Combo($combo,[uint32]$ownedPid)
    if ((Message 2310 0x157) -ne 0) { throw '2310 unexpectedly dropped open' }
    $comboShot=Snapshot 'dark-combo-2310-closed' $combo
    Assert-RenderedContrast $comboShot 'closed combo 2310 selected text' -Region $info.Item -Minimum 4.5 | Out-Null
    $arrowRegion=[ModernNative]::ComboArrowRegion([StatusBarNative]::Client($combo,[uint32]$ownedPid),[ClarityNative]::GetDpiForWindow($combo))
    Record 'custom closed ComboBox arrow region vs native rcButton' @{ RenderedArrow=$arrowRegion; NativeButton=$info.Button; Sampling='Every physical pixel inside border-excluded custom arrow region; 1 DIP chevron'; MinimumContrast=3.0 }
    $arrowShot=Snapshot 'dark-combo-2310-arrow-inspection' $combo -Region $arrowRegion
    Assert-RenderedContrast $arrowShot 'closed combo 2310 arrow' -Region $arrowRegion -Minimum 3.0 -Dense | Out-Null

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
    Assert-RenderedContrast $darkChart 'history chart interior (text/series need visual review)' | Out-Null

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

    $stage='native settings/history Edit drafts and six-category segment'
    Select-Page 1 2300
    $historyEdit=Control 2304
    [ProxyUiSmoke]::SetText($historyEdit,[uint32]$ownedPid,'127.0.0.2')
    Message 2304 0xB1 1 5 | Out-Null
    Select-Category 0
    Wait-State 'upstream URL Edit under viewport' {
        @([ModernNative]::DirectChildren($hostWindow,[uint32]$ownedPid) | Where-Object {
            [ProxyUiSmoke]::Class($_) -eq 'Edit' -and [ProxyUiSmoke]::IsWindowVisible($_) -and
            ([ProxyUiSmoke]::GetWindowLongW($_,-16) -band 0x20) -eq 0 -and
            [ClarityNative]::Text($_,[uint32]$ownedPid) -eq "http://127.0.0.1:$($mock.Port)/v1"
        }).Count -eq 1
    }
    $draftEdit=@([ModernNative]::DirectChildren($hostWindow,[uint32]$ownedPid) | Where-Object {
        [ProxyUiSmoke]::Class($_) -eq 'Edit' -and [ProxyUiSmoke]::IsWindowVisible($_) -and
        ([ProxyUiSmoke]::GetWindowLongW($_,-16) -band 0x20) -eq 0 -and
        [ClarityNative]::Text($_,[uint32]$ownedPid) -eq "http://127.0.0.1:$($mock.Port)/v1"
    })[0]
    $draft="http://127.0.0.1:$($mock.Port)/unsaved-modern-ui-draft"
    [ProxyUiSmoke]::SetText($draftEdit,[uint32]$ownedPid,$draft)
    [ModernNative]::Message($draftEdit,[uint32]$ownedPid,0xB1,7,16) | Out-Null
    $editBaseline=[Collections.Generic.List[object]]::new()
    $editBaseline.Add([ModernNative]::Edit($historyEdit,[uint32]$ownedPid))
    foreach ($h in [ModernNative]::DirectChildren($hostWindow,[uint32]$ownedPid)) {
        $id=[ClarityNative]::GetDlgCtrlID($h)
        if ([ProxyUiSmoke]::Class($h) -eq 'Edit' -and ($id -eq 0 -or ($id -ge 2040 -and $id -le 2061))) {
            $state=[ModernNative]::Edit($h,[uint32]$ownedPid)
            if ($state.Password -and ($state.Length -ne 0 -or $state.Mask -eq 0)) { throw 'Expected empty masked synthetic password field; never read password text' }
            $editBaseline.Add($state)
        }
    }
    if ($editBaseline.Count -lt 5 -or @($editBaseline | Where-Object Password).Count -lt 2) { throw 'Viewport-native settings/password/input fixtures missing; input preservation NOT validated' }
    foreach ($category in 0..5) {
        Select-Category $category
        Assert-PageLayout "settings-category-$category"
        Assert-InputsPreserved "settings category $category"
        Assert-ConfigUnchanged
        Record 'six-category native segment selection and CB compatibility' @{ Category=$category; Native=(Message 2039 0x130B); Compatible=(Message 2039 0x147); Count=(Message 2039 0x146) }
    }
    $stage='three native theme previews, four pages and compact/expanded OFFSCREEN matrix'
    foreach ($index in @(0,1,2)) {
        $name=@('system','light','dark')[$index]
        Select-Category 5
        Select-Segment 2030 $index
        Start-Sleep -Milliseconds 200
        Capture-Segment 2030 $index $name
        Assert-InputsPreserved "theme $name"
        Assert-ConfigUnchanged
        if ($index -eq 0) { continue }
        Select-Page 1 2300
        Wait-State 'history default applied view restored without applying draft filter' { (Visible 2332) -and (Visible 2337) -and (Message 2337 0x1004) -eq 10 }
        $statusShot=Snapshot "$name-status-bar" (Fixed-Footer)
        $statusProof=Assert-RenderedContrast $statusShot "$name status bar text" -Minimum 4.5
        $chart=Snapshot "history-$name-preview-chart" (Control 2332)
        $chartProof=Assert-RenderedContrast $chart "$name chart interior (text/series need visual review)"
        Ensure-Reachable (Control 2337)
        $tablePreview=Snapshot "$name-table-preview" (Control 2337)
        Assert-RenderedContrast $tablePreview "$name table interior" | Out-Null
        $tableProof=Assert-TableContrast "$name native table"
        $head=[IntPtr](Message 2337 0x101F)
        $headerPreview=Snapshot "$name-table-header-preview" $head
        Assert-RenderedContrast $headerPreview "$name native header text" -Minimum 4.5 | Out-Null
        Ensure-Reachable (Control 2335)
        $comboPreview=Snapshot "$name-combo-preview" (Control 2335)
        $comboInfo=[ModernNative]::Combo((Control 2335),[uint32]$ownedPid)
        Assert-RenderedContrast $comboPreview "$name closed sort combo text" -Region $comboInfo.Item -Minimum 4.5 | Out-Null
        Assert-HistoryLayout "history-$name-preview"
        if ($name -eq 'light') { $lightChart=$chartProof; $lightTable=$tableProof; $lightStatus=$statusProof }
        else {
            Assert-ThemePair $lightChart $chartProof 'history chart'
            Assert-ThemePair $lightTable $tableProof 'native table'
            Assert-ThemePair $lightStatus $statusProof 'fixed status bar'
        }
        Select-Page 2 2228
        Ensure-Reachable (Control 2228)
        Snapshot "$name-switch-2228-off" (Control 2228) | Out-Null
        Message 2228 0xF5 | Out-Null
        Wait-State "$name switch on" { (Message 2228 0xF0) -eq 1 }
        Snapshot "$name-switch-2228-on" (Control 2228) | Out-Null
        Message 2228 0xF5 | Out-Null
        Wait-State "$name switch restored off" { (Message 2228 0xF0) -eq 0 }
        # The two-size page matrix also covers all six settings categories and
        # layout-only history subviews at TOP/BOTTOM plus cached return.
        Capture-PageMatrix $name
        Select-Category 5
        if ((Message 2030 0x147) -ne $index) { throw 'Page/category navigation reset theme selection' }
        Assert-InputsPreserved "$name complete matrix"
        Assert-ConfigUnchanged
    }
    Select-Category 0
    Wait-State 'draft visible and retained after theme/page/scroll/resize changes' {
        [ProxyUiSmoke]::IsWindowVisible($draftEdit) -and [ClarityNative]::Text($draftEdit,[uint32]$ownedPid) -ceq $draft
    }
    Assert-InputsPreserved 'final'
    if ($matrix.Count -ne 16) { throw 'Four-page, two-size, light/dark offscreen evidence matrix incomplete; expected 16 top captures' }
    if ($settingsMatrix.Count -ne 24 -or $historyTailMatrix.Count -ne 24) { throw 'Six-category/history-tail TOP/BOTTOM/cache matrix incomplete; expected 24 entries each (two themes x two sizes x six views)' }
    Assert-ConfigUnchanged
    Record 'native Edit values/selection/styles/masks/limits/margins and no-region contract retained; theme preview never saved config' @{ Edits=$editBaseline.Count; MatrixEntries=$matrix.Count; SettingsTailEntries=$settingsMatrix.Count; HistoryTailEntries=$historyTailMatrix.Count; PasswordScope='Empty masked fields only, no nonempty-secret retention claim' }
    if ($mock.Requests -ne 10) { throw 'Unexpected extra network requests during theme/history testing' }
    if ($uiFailures.Count -gt 0) { throw ("UI layout/contrast evidence failures:`n"+($uiFailures -join "`n")) }
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
        Passed=($null -eq $failure -and $cleanupErrors.Count -eq 0 -and $uiFailures.Count -eq 0); Exe=$exe; PID=$ownedPid;
        Failure=$detail; UIFailures=@($uiFailures.ToArray()); CleanupErrors=@($cleanupErrors.ToArray());
        ExpectedPageMatrixEntries=16; PageMatrixComplete=($matrix.Count -eq 16);
        ExpectedSettingsTailEntries=24; SettingsTailMatrixComplete=($settingsMatrix.Count -eq 24); SettingsTailMatrix=@($settingsMatrix.ToArray());
        ExpectedHistoryTailEntries=24; HistoryTailMatrixComplete=($historyTailMatrix.Count -eq 24); HistoryTailMatrix=@($historyTailMatrix.ToArray());
        ManagementTailCoverage='2325 complete-tail assertion supported; GUI entry NOT exercised because 2339 starts HistoryStorage disk I/O, not a layout-only switch';
        Checks=@($checks.ToArray()); Screenshots=@($shots.ToArray()); PageMatrix=@($matrix.ToArray()); ReadabilityEvidence=@($readability.ToArray()); Seconds=[Math]::Round($overall.Elapsed.TotalSeconds,2);
        CaptureKind='All PNGs are PrintWindow OFFSCREEN rendering at physical-pixel dimensions, NOT desktop/screen/occlusion/residual evidence';
        VisualReadabilityReview='NOT RUN by this script: manually inspect every page, text, legend, disabled state and clipped/scrolling area in the recorded PNGs; interior color evidence is not OCR or every-glyph proof';
        Limits='Current monitor actual DPI only; requested 800x680/1000x820 outer DIP sizes may be work-area-clamped (recorded), not an interactive minimum-track or multi-DPI test. No high-contrast switch, screen-reader, physical keyboard/tray or long-run resource acceptance. No fixed legacy Palette RGB, uniform-white-tile or per-item blue-border requirement. Passwords start empty; only mask/length/style/selection retention checked. No Cargo, real upstream/key/data, clipboard access, Explorer, Startup/firewall/system preference changes, config Save, global input or IPC to another instance. Pending is observed, never artificially delayed.'
    }
    [IO.File]::WriteAllText((Join-Path $output 'results.json'),($report | ConvertTo-Json -Depth 12),[Text.UTF8Encoding]::new($true))
    Write-Output "Modern UI artifacts: $output"
}
if ($null -ne $failure) { throw $failure }
if ($cleanupErrors.Count -gt 0) { throw ($cleanupErrors -join '; ') }
Write-Output 'PASS: six-category/native segmented clicks and CB compatibility, real loopback queries/pending, viewport/document/full-tail 16-DIP padding and content-sized settings extents, native switches/Edit draft retention, relative light/dark and scoped contrast evidence, four-page/two-size and six-category/history-subview TOP/BOTTOM/cache PrintWindow OFFSCREEN matrices and owned-child cleanup. Management entry (storage I/O), physical-screen and complete visual readability acceptance NOT performed.'
