# Windows PowerShell 5.1 x64 / Windows 10、11 交互桌面。
# 仅测试自己启动的进程；不构建、不联网、不调用旧脚本，不操作 Startup/剪贴板。
# 先运行 -ValidateOnly。默认 EXE 待构建；没有构建通知时不要运行 GUI body。
# 示例：powershell.exe -NoProfile -File tools/smoke-title-menu-ui.ps1 -ValidateOnly
[CmdletBinding()]
param(
    [string]$ExePath,
    [switch]$ValidateOnly,

    [ValidateRange(60,300)][int]$BudgetSeconds = 180
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([IntPtr]::Size -ne 8) { throw 'Use fresh x64 powershell.exe -NoProfile' }
if ('TitleMenuSmoke.Native' -as [type]) { throw 'Helper already loaded; use a fresh PowerShell process' }
if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $PSScriptRoot '..\target\title-menu-build\debug\ai-relay.exe'
}

# 独立声明，不提取/执行其他 smoke。WM_USER 以上只发送标量；无远程分配、
# 注入、WM_NOTIFY、本地指针跨进程消息或“直接发页面 action”冒充菜单选择。
$nativeSource = @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Threading.Tasks;
namespace TitleMenuSmoke {
public static class Native {
    [StructLayout(LayoutKind.Sequential)] public struct Rect {
        public int Left,Top,Right,Bottom;
        public int Width { get { return Right-Left; } }
        public int Height { get { return Bottom-Top; } }
        public override string ToString() { return String.Format("{0},{1} {2}x{3}",Left,Top,Width,Height); }
    }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
    [StructLayout(LayoutKind.Sequential)] struct MenuInfo {
        public uint Size,Mask,Type,State,Id;
        public IntPtr Submenu,Checked,Unchecked,Data,Text;
        public uint Length; public IntPtr Bitmap;
    }
    [StructLayout(LayoutKind.Sequential)] struct GuiInfo {
        public uint Size,Flags; public IntPtr Active,Focus,Capture,MenuOwner,MoveSize,Caret; public Rect CaretRect;
    }
    [StructLayout(LayoutKind.Sequential)] struct ScrollInfo {
        public uint Size,Mask; public int Min,Max; public uint Page; public int Position,Track;
    }
    [StructLayout(LayoutKind.Sequential)] struct TitleInfo {
        public uint Size; public Rect Bounds; public uint State0,State1,State2,State3,State4,State5;
    }
    [StructLayout(LayoutKind.Sequential)] struct IconInfo {
        public int IsIcon; public uint X,Y; public IntPtr Mask,Color;
    }
    public sealed class MenuItem {
        public int Position; public uint Id,State,Type; public string Text; public IntPtr Submenu;
        public bool Enabled { get { return (State&3)==0; } }
        public bool Checked { get { return (State&8)!=0; } }
        public bool Highlighted { get { return (State&0x80)!=0; } }
    }
    public sealed class IconResult { public int Width,Height,Bands; public string Hash; }
    delegate bool EnumProc(IntPtr h,IntPtr unused);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback,IntPtr unused);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent,EnumProc callback,IntPtr unused);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h,StringBuilder text,int size);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);
    [DllImport("user32.dll")] static extern int GetWindowLongW(IntPtr h,int index);
        [DllImport("user32.dll",EntryPoint="GetClassLongPtrW")] static extern IntPtr GetClassLongPtrW(IntPtr h,int index);
    [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h,uint relation);
    [DllImport("user32.dll")] public static extern IntPtr GetParent(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetMenu(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point p);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h,out Rect r);
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h,out Rect r);
    [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h,ref Point p);
    [DllImport("user32.dll")] static extern bool GetTitleBarInfo(IntPtr h,ref TitleInfo info);
    [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint thread,ref GuiInfo info);
    [DllImport("user32.dll")] static extern bool GetScrollInfo(IntPtr h,int bar,ref ScrollInfo info);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] static extern int GetSystemMetricsForDpi(int index,uint dpi);
    [DllImport("user32.dll")] static extern bool AdjustWindowRectExForDpi(ref Rect r,uint style,bool menu,uint exStyle,uint dpi);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern int GetAwarenessFromDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern IntPtr GetThreadDpiAwarenessContext();
    [DllImport("user32.dll")] static extern IntPtr GetSystemMenu(IntPtr h,bool revert);
    [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr menu);
    [DllImport("user32.dll")] static extern IntPtr GetSubMenu(IntPtr menu,int position);
    [DllImport("user32.dll")] static extern uint GetMenuItemID(IntPtr menu,int position);
    [DllImport("user32.dll")] static extern uint GetMenuState(IntPtr menu,uint item,uint flags);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern bool GetMenuItemInfoW(IntPtr menu,uint position,bool byPosition,ref MenuInfo info);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetMenuStringW(IntPtr menu,uint item,StringBuilder text,int capacity,uint flags);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr h,uint message,UIntPtr w,IntPtr l,uint flags,uint timeout,out UIntPtr result);
    [DllImport("user32.dll",CharSet=CharSet.Unicode,EntryPoint="SendMessageTimeoutW")] static extern IntPtr ReadText(IntPtr h,uint message,UIntPtr w,StringBuilder l,uint flags,uint timeout,out UIntPtr result);
    [DllImport("user32.dll",CharSet=CharSet.Unicode,EntryPoint="SendMessageTimeoutW")] static extern IntPtr WriteText(IntPtr h,uint message,UIntPtr w,string l,uint flags,uint timeout,out UIntPtr result);
    [DllImport("user32.dll",SetLastError=true)] static extern bool PostMessageW(IntPtr h,uint message,UIntPtr w,IntPtr l);
    [DllImport("user32.dll")] static extern uint MapVirtualKeyW(uint key,uint kind);
    [DllImport("user32.dll")] static extern uint GetGuiResources(IntPtr process,uint flags);
    [DllImport("kernel32.dll")] static extern uint GetProcessId(IntPtr process);
    [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr process,uint milliseconds);
    [DllImport("user32.dll")] static extern IntPtr CopyIcon(IntPtr icon);
    [DllImport("user32.dll")] static extern bool DestroyIcon(IntPtr icon);
    [DllImport("user32.dll")] static extern bool GetIconInfo(IntPtr icon,out IconInfo info);
    [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr value);
    [DllImport("shell32.dll",CharSet=CharSet.Unicode)] static extern uint ExtractIconExW(string file,int index,IntPtr[] large,IntPtr[] small,uint count);
    [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h,IntPtr dc,uint flags);

    public static string Validate() {
        if(IntPtr.Size!=8 || Marshal.SizeOf(typeof(MenuInfo))!=80 || Marshal.SizeOf(typeof(GuiInfo))!=72 ||
           Marshal.SizeOf(typeof(TitleInfo))!=44 || Marshal.SizeOf(typeof(ScrollInfo))!=28 || Marshal.SizeOf(typeof(IconInfo))!=32)
            throw new Exception("Unexpected x64 Win32 layouts");
        if(Pack(new Point { X=-12,Y=345 })!=unchecked((int)0x0159FFF4)) throw new Exception("Signed screen-coordinate packing failed");
        return "x64 MENUITEMINFO=80 GUITHREADINFO=72 TITLEBARINFO=44 SCROLLINFO=28 ICONINFO=32; signed coordinates";
    }
    public static uint Owner(IntPtr h) { uint pid; GetWindowThreadProcessId(h,out pid); return pid; }
    public static void Check(IntPtr h,uint pid) {
        if(h==IntPtr.Zero || pid==0 || Owner(h)!=pid) throw new Exception("Foreign/missing HWND; refused");
    }
    public static void CheckProcess(IntPtr handle,uint pid) {
        if(handle==IntPtr.Zero || GetProcessId(handle)!=pid || WaitForSingleObject(handle,0)!=258)
            throw new Exception("Pinned child process no longer live");
    }
    public static string Class(IntPtr h) { var b=new StringBuilder(256); GetClassNameW(h,b,b.Capacity); return b.ToString(); }
    public static IntPtr FindMain(uint pid) {
        IntPtr found=IntPtr.Zero;
        EnumWindows(delegate(IntPtr h,IntPtr unused) { if(Owner(h)==pid && Class(h)=="AIRelay.Window.v1") { found=h; return false; } return true; },IntPtr.Zero);
        return found;
    }
    public static IntPtr[] Popups(uint pid,string cls) {
        var list=new List<IntPtr>();
        EnumWindows(delegate(IntPtr h,IntPtr unused) { if(Owner(h)==pid && IsWindowVisible(h) && Class(h)==cls) list.Add(h); return true; },IntPtr.Zero);
        return list.ToArray();
    }
    public static IntPtr[] Children(IntPtr parent,uint pid,bool recursive) {
        Check(parent,pid); var list=new List<IntPtr>(); bool invalid=false;
        if(recursive) EnumChildWindows(parent,delegate(IntPtr h,IntPtr unused) {
            if(list.Count>=4096 || Owner(h)!=pid) { invalid=true; return false; } list.Add(h); return true;
        },IntPtr.Zero);
        else for(IntPtr h=GetWindow(parent,5);h!=IntPtr.Zero;h=GetWindow(h,2)) {
            if(list.Count>=4096 || Owner(h)!=pid) { invalid=true; break; } list.Add(h);
        }
        if(invalid) throw new Exception("Foreign/excessive child windows"); return list.ToArray();
    }
    public static IntPtr FindControl(IntPtr main,uint pid,int id) {
        IntPtr found=IntPtr.Zero;
        foreach(var h in Children(main,pid,true)) if(GetDlgCtrlID(h)==id) {
            if(found!=IntPtr.Zero) throw new Exception("Duplicate control ID "+id); found=h;
        }
        return found;
    }
    public static long Message(IntPtr h,uint pid,uint message,ulong w,long l) {
        Check(h,pid); UIntPtr result;
        if(SendMessageTimeoutW(h,message,new UIntPtr(w),new IntPtr(l),0x22,1000,out result)==IntPtr.Zero)
            throw new Exception("Bounded message timed out: 0x"+message.ToString("X"));
        return unchecked((long)result.ToUInt64());
    }
    public static void Post(IntPtr h,uint pid,uint message,ulong w,long l) {
        Check(h,pid);
        if(!PostMessageW(h,message,new UIntPtr(w),new IntPtr(l))) throw new Exception("Post failed: "+Marshal.GetLastWin32Error());
    }
    public static string Text(IntPtr h,uint pid) {
        Check(h,pid);
        if(Class(h)=="Edit" && (GetWindowLongW(h,-16)&0x20)!=0) throw new Exception("Password read denied");
        var b=new StringBuilder(8192); UIntPtr result;
        // WM_GETTEXT < WM_USER: user32 marshals the caller-owned buffer.
        if(ReadText(h,13,new UIntPtr((uint)b.Capacity),b,0x22,1000,out result)==IntPtr.Zero) throw new Exception("WM_GETTEXT timed out");
        return b.ToString();
    }
    public static void SetText(IntPtr h,uint pid,string value) {
        Check(h,pid);
        if(Class(h)!="Edit" || (GetWindowLongW(h,-16)&0x20)!=0) throw new Exception("Only non-password fixture Edit is writable");
        UIntPtr result;
        if(WriteText(h,12,UIntPtr.Zero,value,0x22,1000,out result)==IntPtr.Zero || result==UIntPtr.Zero) throw new Exception("WM_SETTEXT failed");
    }
    public static void Key(IntPtr main,uint pid,uint key,bool alt,bool system) {
        Check(main,pid);
        uint bits=1u|(MapVirtualKeyW(key,0)<<16);
        if(key>=0x21 && key<=0x28) bits|=0x01000000;
        if(alt) bits|=0x20000000;
        Post(main,pid,(alt||system)?0x104u:0x100u,key,unchecked((int)bits));
        Post(main,pid,(alt||system)?0x105u:0x101u,key,unchecked((int)(bits|0xC0000000u)));
    }
    public static bool InMenu(IntPtr main,uint pid) {
        Check(main,pid); uint actual; uint thread=GetWindowThreadProcessId(main,out actual);
        var info=new GuiInfo { Size=(uint)Marshal.SizeOf(typeof(GuiInfo)) };
        if(!GetGUIThreadInfo(thread,ref info)) throw new Exception("GetGUIThreadInfo failed");
        if((info.Flags&0x1Cu)==0) return false;
        if(info.MenuOwner!=IntPtr.Zero) Check(info.MenuOwner,pid);
        return true;
    }
    public static IntPtr Root(IntPtr main,uint pid) {
        Check(main,pid); var root=GetSystemMenu(main,false);
        if(root==IntPtr.Zero) throw new Exception("Missing native system menu"); return root;
    }
    public static MenuItem[] Items(IntPtr main,uint pid,IntPtr menu) {
        Check(main,pid); var root=Root(main,pid); bool allowed=menu==root;
        for(int i=0;i<4;i++) if(GetSubMenu(root,i)==menu && menu!=IntPtr.Zero) allowed=true;
        if(!allowed) throw new Exception("HMENU is not the owned root/four submenus");
        int count=GetMenuItemCount(menu); if(count<0 || count>64) throw new Exception("Invalid/excessive menu count");
        var list=new List<MenuItem>();
        for(uint i=0;i<count;i++) {
            // Public APIs copy kernel/menu data into caller buffers, even for a
            // foreign process's HMENU. No MENUITEMINFO pointer is sent as a message.
            var info=new MenuInfo { Size=(uint)Marshal.SizeOf(typeof(MenuInfo)),Mask=0x107 };
            if(!GetMenuItemInfoW(menu,i,true,ref info)) throw new Exception("GetMenuItemInfo failed");
            var text=new StringBuilder(512); int length=GetMenuStringW(menu,i,text,text.Capacity,0x400);
            if(length>=text.Capacity-1) throw new Exception("Menu string truncated");
            IntPtr sub=GetSubMenu(menu,(int)i);
            if(sub!=info.Submenu || (sub==IntPtr.Zero && GetMenuItemID(menu,(int)i)!=info.Id)) throw new Exception("Menu ID/submenu query mismatch");
            if(GetMenuState(menu,i,0x400)==UInt32.MaxValue) throw new Exception("GetMenuState failed");
            list.Add(new MenuItem { Position=(int)i,Id=info.Id,State=info.State,Type=info.Type,Text=text.ToString(),Submenu=sub });
        }
        return list.ToArray();
    }
    public static Rect Outer(IntPtr h,uint pid) { Check(h,pid); Rect r; if(!GetWindowRect(h,out r)) throw new Exception("GetWindowRect failed"); return r; }
    public static Rect Client(IntPtr h,uint pid) { Check(h,pid); Rect r; if(!GetClientRect(h,out r)) throw new Exception("GetClientRect failed"); return r; }
    public static Point Origin(IntPtr h,uint pid) { Check(h,pid); var p=new Point(); if(!ClientToScreen(h,ref p)) throw new Exception("ClientToScreen failed"); return p; }
    public static int Offset(IntPtr h,uint pid) {
        Check(h,pid); var s=new ScrollInfo { Size=28,Mask=0x17 };
        if(!GetScrollInfo(h,1,ref s)) throw new Exception("GetScrollInfo failed"); return s.Position;
    }
    public static string Frame(IntPtr main,uint pid) {
        Check(main,pid); var outer=Outer(main,pid); var client=Client(main,pid); var adjusted=client;
        if(GetMenu(main)!=IntPtr.Zero || !AdjustWindowRectExForDpi(ref adjusted,unchecked((uint)GetWindowLongW(main,-16)),false,
            unchecked((uint)GetWindowLongW(main,-20)),GetDpiForWindow(main))) throw new Exception("Unexpected menu bar/native frame query failure");
        if(outer.Width!=adjusted.Width || outer.Height!=adjusted.Height) throw new Exception("Client/frame dimensions differ from native no-menu frame");
        return "outer="+outer+"; client="+client+"; no-menu AdjustWindowRectExForDpi="+adjusted;
    }
    public static int Pack(Point p) { return unchecked((int)(((uint)p.X&0xFFFFu)|(((uint)p.Y&0xFFFFu)<<16))); }
    public static Point CaptionIcon(IntPtr main,uint pid) {
        Check(main,pid); var r=Outer(main,pid); uint dpi=GetDpiForWindow(main);
        var title=new TitleInfo { Size=44 }; if(!GetTitleBarInfo(main,ref title)) throw new Exception("GetTitleBarInfo failed");
        int border=GetSystemMetricsForDpi(32,dpi)+GetSystemMetricsForDpi(92,dpi);
        int width=GetSystemMetricsForDpi(49,dpi);
        int[] xs={ r.Left+border+width/2,title.Bounds.Left+width/2,r.Left+(int)(16*dpi/96) };
        int[] ys={ title.Bounds.Top+title.Bounds.Height/2,r.Top+border+GetSystemMetricsForDpi(4,dpi)/2 };
        foreach(int x in xs) foreach(int y in ys) {
            var point=new Point { X=x,Y=y };
            if(x<Int16.MinValue || x>Int16.MaxValue || y<Int16.MinValue || y>Int16.MaxValue) continue;
            if(Message(main,pid,0x84,0,Pack(point))==3) return point;
        }
        throw new Exception("No verified HTSYSMENU icon center; no click sent");
    }
    public static void Foreground(IntPtr main,uint pid) {
        Check(main,pid); SetForegroundWindow(main);
        if(Owner(GetForegroundWindow())!=pid) throw new Exception("Foreground denied; do not steal focus/input");
    }
    public static uint Resources(IntPtr process,uint pid,uint flag) {
        CheckProcess(process,pid); uint value=GetGuiResources(process,flag);
        if(value==0) throw new Exception("GetGuiResources failed"); return value;
    }
    public static IntPtr EnterDpi() {
        IntPtr old=SetThreadDpiAwarenessContext(new IntPtr(-4));
        if(old==IntPtr.Zero || GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext())!=2) throw new Exception("Physical PMv2 coordinates unavailable");
        return old;
    }
    public static IconResult IconProof(IntPtr borrowed,string path) {
        if(borrowed==IntPtr.Zero) throw new Exception("Null icon");
        IntPtr copy=CopyIcon(borrowed); if(copy==IntPtr.Zero) throw new Exception("CopyIcon failed");
        try {
            IconInfo info; if(!GetIconInfo(copy,out info)) throw new Exception("GetIconInfo failed");
            try {
                using(var icon=Icon.FromHandle(copy)) using(var b=icon.ToBitmap()) {
                    if(b.Width>256 || b.Height>256) throw new Exception("Oversize icon");
                    int bands=0; bool previous=false;
                    for(int y=0;y<b.Height;y++) {
                        int opaque=0; for(int x=0;x<b.Width;x++) if(b.GetPixel(x,y).A>=200) opaque++;
                        bool stroke=opaque>=b.Width/2; if(stroke && !previous) bands++; previous=stroke;
                    }
                    if(path!=null) b.Save(path,ImageFormat.Png);
                    using(var normalized=new Bitmap(32,32,PixelFormat.Format32bppArgb)) {
                        using(var g=Graphics.FromImage(normalized)) { g.Clear(Color.Transparent); g.DrawIcon(icon,new Rectangle(0,0,32,32)); }
                        using(var stream=new MemoryStream()) using(var hash=SHA256.Create()) {
                            normalized.Save(stream,ImageFormat.Png);
                            return new IconResult { Width=b.Width,Height=b.Height,Bands=bands,Hash=Convert.ToBase64String(hash.ComputeHash(stream.ToArray())) };
                        }
                    }
                }
            } finally { if(info.Mask!=IntPtr.Zero) DeleteObject(info.Mask); if(info.Color!=IntPtr.Zero) DeleteObject(info.Color); }
        } finally { DestroyIcon(copy); }
    }
    public static IntPtr EffectiveIcon(IntPtr main,uint pid,bool big) {
            // WM_GETICON may return null when WNDCLASSEX supplies the icon instead.
            IntPtr icon=new IntPtr(Message(main,pid,0x7F,big?1u:0u,0));
            if(icon==IntPtr.Zero) icon=GetClassLongPtrW(main,big?-14:-34);
            if(icon==IntPtr.Zero) throw new Exception("No window or class icon"); return icon;
        }
        public static string ExecutableIcon(string exe) {
        var large=new IntPtr[1]; var small=new IntPtr[1];
        try {
            if(ExtractIconExW(exe,0,large,small,1)==0 || large[0]==IntPtr.Zero) throw new Exception("No executable app icon to compare");
            return IconProof(large[0],null).Hash;
        } finally { if(large[0]!=IntPtr.Zero) DestroyIcon(large[0]); if(small[0]!=IntPtr.Zero) DestroyIcon(small[0]); }
    }
    static void SizeBound(Rect r) {
        if(r.Width<8 || r.Height<8 || r.Width>16384 || r.Height>16384 || (long)r.Width*r.Height>64000000) throw new Exception("Invalid/excessive capture size");
    }
    public static string Capture(IntPtr h,uint pid,string path,bool popup) {
        Check(h,pid); Rect r=Outer(h,pid); SizeBound(r);
        if(!IsWindowVisible(h)) throw new Exception("Capture target hidden");
        if(popup) {
            if(Class(h)!="#32768" || Owner(GetForegroundWindow())!=pid) throw new Exception("Only foreground owned native menu may be screen-captured");
            // Capture ONLY the native menu rectangle, never the desktop. Refuse
            // occluded sample points instead of collecting another application's UI.
            foreach(int x in new int[] { r.Left+2,r.Left+r.Width/2,r.Right-3 })
                foreach(int y in new int[] { r.Top+2,r.Top+r.Height/2,r.Bottom-3 })
                    if(Owner(WindowFromPoint(new Point { X=x,Y=y }))!=pid) throw new Exception("Menu obscured; screenshot refused");
            using(var b=new Bitmap(r.Width,r.Height,PixelFormat.Format32bppArgb)) {
                using(var g=Graphics.FromImage(b)) g.CopyFromScreen(r.Left,r.Top,0,0,b.Size,CopyPixelOperation.SourceCopy);
                if(Owner(GetForegroundWindow())!=pid || !IsWindowVisible(h)) throw new Exception("Menu changed during screenshot");
                b.Save(path,ImageFormat.Png);
            }
        } else {
            // PrintWindow is synchronous; isolate the documented API in a bounded
            // worker which owns its DC/bitmap until completion, even after timeout.
            var task=Task.Run(delegate {
                using(var b=new Bitmap(r.Width,r.Height,PixelFormat.Format32bppArgb)) {
                    using(var g=Graphics.FromImage(b)) {
                        IntPtr dc=g.GetHdc(); bool ok;
                        try { ok=PrintWindow(h,dc,2); } finally { g.ReleaseHdc(dc); }
                        if(!ok) throw new Exception("PrintWindow failed");
                    }
                    using(var stream=new MemoryStream()) { b.Save(stream,ImageFormat.Png); return stream.ToArray(); }
                }
            });
            if(!task.Wait(2000)) throw new Exception("PrintWindow exceeded 2s; no capture saved");
            File.WriteAllBytes(path,task.Result);
        }
        return r.ToString();
    }
}
}
'@
Add-Type -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll -TypeDefinition $nativeSource
$layoutValidation = [TitleMenuSmoke.Native]::Validate()
# 编译 C# 不会校验 PowerShell 的静态成员调用；在 validate 阶段一起验证，
# 防止私有 P/Invoke 只在 About/清理等后段才暴露脚本自身错误。
$tokens=$null; $parseErrors=$null
$scriptAst=[Management.Automation.Language.Parser]::ParseFile($PSCommandPath,[ref]$tokens,[ref]$parseErrors)
if ($parseErrors.Count -ne 0) { throw 'PowerShell parser errors in this script' }
$nativeCalls=$scriptAst.FindAll({ param($node)
    $node -is [Management.Automation.Language.InvokeMemberExpressionAst] -and
    $node.Static -and $node.Expression.Extent.Text -eq '[TitleMenuSmoke.Native]'
},$true)
foreach ($call in $nativeCalls) {
    $method=$call.Member.Value
    if (@([TitleMenuSmoke.Native].GetMethods() | Where-Object { $_.IsStatic -and $_.Name -eq $method }).Count -eq 0) {
        throw "PowerShell references missing/non-public native helper: $method"
    }
}
if ($ValidateOnly) {
    Write-Output "PASS: PowerShell AST, public static helper call sites, C# compilation and $layoutValidation"
    Write-Output 'VALIDATION ONLY: no EXE lookup/build, mutex, files, process launch, GUI, network, clipboard or acceptance checks'
    return
}

$session = [Diagnostics.Process]::GetCurrentProcess().SessionId
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$mutexName = "Local\AIRelay.$sid.$session.v1"
$exe = [IO.Path]::GetFullPath($ExePath)
$processNames = @('ai-relay',[IO.Path]::GetFileNameWithoutExtension($exe)) | Select-Object -Unique
$child=$null; $childHandle=[IntPtr]::Zero; $childStart=$null; $ownedPid=0
$window=[IntPtr]::Zero; $root=[IntPtr]::Zero; $temp=$null; $output=$null
$oldDpi=[IntPtr]::Zero; $failure=$null; $normalExit=$false; $stage='preflight'
$checks=[Collections.Generic.List[object]]::new()
$shots=[Collections.Generic.List[object]]::new()
$resources=[Collections.Generic.List[object]]::new()
$cleanupErrors=[Collections.Generic.List[string]]::new()
$uiFailures=[Collections.Generic.List[string]]::new()
$clock=[Diagnostics.Stopwatch]::StartNew()
function Record([string]$Name,[object]$Value) {
    $checks.Add([pscustomobject]@{ Check=$Name; Value=$Value; Seconds=[Math]::Round($clock.Elapsed.TotalSeconds,2) })
    Write-Host ("CHECK {0}: {1}" -f $Name,($Value | ConvertTo-Json -Compress -Depth 6))
}
function Assert-NoOtherProcess {
    $others=@(Get-Process -Name $processNames -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $ownedPid })
    if ($others.Count -gt 0) { throw "BLOCKED: foreign app PID(s) $($others.Id -join ','); no IPC/input/kill to them" }
}
function Assert-NoInstance {
    $existing=$null
    try { $existing=[Threading.Mutex]::OpenExisting($mutexName) }
    catch [Threading.WaitHandleCannotBeOpenedException] { }
    if ($null -ne $existing) { $existing.Dispose(); throw 'BLOCKED: existing user/session mutex; no launch/IPC' }
    Assert-NoOtherProcess
}
function Assert-Identity {
    if ($null -eq $child -or $child.HasExited) { throw 'Owned child missing/exited; input refused' }
    [TitleMenuSmoke.Native]::CheckProcess($childHandle,[uint32]$ownedPid)
    if ($child.Id -ne $ownedPid -or $child.Handle -ne $childHandle -or $child.StartTime -ne $childStart -or $child.Path -ine $exe) { throw 'Pinned PID/handle/start-time/exe mismatch' }
}
function Guard {
    Assert-NoOtherProcess
    Assert-Identity
    if ($clock.Elapsed.TotalSeconds -ge $BudgetSeconds) { throw "Overall ${BudgetSeconds}s budget exceeded at $stage" }
}
function Wait-State([string]$Name,[scriptblock]$Test,[int]$Milliseconds=4000) {
    $timer=[Diagnostics.Stopwatch]::StartNew()
    while ($timer.ElapsedMilliseconds -lt $Milliseconds) {
        Guard
        if (& $Test) { return }
        Start-Sleep -Milliseconds 25
    }
    throw "Wait expired at $stage / $Name (${Milliseconds}ms)"
}
function Control([int]$Id) {
    Guard
    $h=[TitleMenuSmoke.Native]::FindControl($window,[uint32]$ownedPid,$Id)
    if ($h -eq [IntPtr]::Zero) { throw "Missing recursive control $Id; build latest title-menu UI first" }
    return $h
}
function Message([IntPtr]$Handle,[uint32]$Code,[uint64]$W=0,[long]$L=0) {
    Guard
    return [TitleMenuSmoke.Native]::Message($Handle,[uint32]$ownedPid,$Code,$W,$L)
}
function Post([IntPtr]$Handle,[uint32]$Code,[uint64]$W=0,[long]$L=0) {
    Guard
    [TitleMenuSmoke.Native]::Post($Handle,[uint32]$ownedPid,$Code,$W,$L)
}
function Key([uint32]$Code,[switch]$Alt,[switch]$System) {
    Guard
    $target=$window
        # Menu tracking has its own HWND/message filter; navigation must target the
        # live native popup (topmost submenu first), not the application's main HWND.
        $menus=@([TitleMenuSmoke.Native]::Popups([uint32]$ownedPid,'#32768'))
        if ($menus.Count -gt 0 -and $Code -in @(0x1B,0x24,0x25,0x26,0x27,0x28,0x0D)) { $target=$menus[0] }
        [TitleMenuSmoke.Native]::Key($target,[uint32]$ownedPid,$Code,[bool]$Alt,[bool]$System)
}
function Items([IntPtr]$Menu) {
    Guard
    return [TitleMenuSmoke.Native]::Items($window,[uint32]$ownedPid,$Menu)
}
function Text([IntPtr]$Handle) { Guard; [TitleMenuSmoke.Native]::Text($Handle,[uint32]$ownedPid) }
function Assert-NoDialog {
    Guard
    if (@([TitleMenuSmoke.Native]::Popups([uint32]$ownedPid,'#32770')).Count -ne 0) { throw 'Unexpected own dialog; never automatically confirmed' }
}
function Capture([string]$Name,[IntPtr]$Handle,[switch]$Popup) {
    Guard
    $path=Join-Path $output ($Name+'.png')
    $bounds=[TitleMenuSmoke.Native]::Capture($Handle,[uint32]$ownedPid,$path,[bool]$Popup)
    $shots.Add([pscustomobject]@{ Name=$Name; Path=$path; PhysicalBounds=$bounds; DPI=[TitleMenuSmoke.Native]::GetDpiForWindow($Handle) })
}
function Check-Menu {
    $entries=@(Items $root)
    if ($entries.Count -lt 9) { throw 'Native system menu was replaced/truncated' }
    $labels=@('文件(&F)','代理(&P)','视图(&V)','帮助(&H)')
    $expected=@(@(0x3000,0x3010,0x3020,0x3030),@(0x3040,0x3050,0x3060),@(0x3070,0x3080,0x3090,0x30A0),@(0x30B0))
    for ($index=0;$index -lt 4;$index++) {
        if ($entries[$index].Text -cne $labels[$index] -or $entries[$index].Submenu -eq [IntPtr]::Zero) { throw "Root order/text/submenu mismatch at $index" }
        $leaves=@(Items $entries[$index].Submenu)
        if (($leaves.Id -join ',') -cne ($expected[$index] -join ',') -or @($leaves | Where-Object { $_.Submenu -ne [IntPtr]::Zero -or [string]::IsNullOrEmpty($_.Text) }).Count -ne 0) { throw "Submenu IDs/text mismatch at $index" }
    }
    if (($entries[4].Type -band 0x800) -eq 0) { throw 'Missing separator at root position 4' }
    foreach ($id in @(0xF120,0xF030,0xF020,0xF060)) {
        if (@($entries | Select-Object -Skip 5 | Where-Object { $_.Id -eq $id }).Count -ne 1) { throw ('Missing/duplicate preserved native SC command 0x{0:X}' -f $id) }
    }
    return $entries
}
function Check-Availability {
    $entries=@(Check-Menu)
    $proxy=@(Items $entries[1].Submenu); $file=@(Items $entries[0].Submenu)
    $live=@(
        @(0x3040,2102,$proxy[0]),@(0x3050,2103,$proxy[1]),@(0x3060,2104,$proxy[2]),
        @(0x3010,2034,$file[1]),@(0x3020,2035,$file[2])
    )
    foreach ($row in $live) {
        $enabled=[TitleMenuSmoke.Native]::IsWindowEnabled((Control $row[1]))
        if ($row[2].Enabled -ne $enabled) { throw ('Live availability mismatch 0x{0:X}, button {1}' -f $row[0],$row[1]) }
    }
    if (-not $proxy[0].Enabled -or $proxy[1].Enabled -or -not $file[1].Enabled -or -not $file[2].Enabled) { throw 'Stopped/ready start-stop/export-import states incorrect' }
    $selection=Message (Control 2100) 0x130B
    $view=@(Items $entries[2].Submenu)
    for ($i=0;$i -lt 4;$i++) { if ($view[$i].Checked -ne ($i -eq $selection)) { throw "View radio state mismatch at $i" } }
}
function Open-Menu([string]$Method='F10') {
    Guard
    Assert-NoDialog
    if ([TitleMenuSmoke.Native]::InMenu($window,[uint32]$ownedPid)) { throw 'Menu already tracking; no nested open' }
    [TitleMenuSmoke.Native]::Foreground($window,[uint32]$ownedPid)
    switch ($Method) {
        'F10' { Key 0x79 }
        'F10System' { Key 0x79 -System } # Real F10 SYSKEY has context bit 29 clear.
        'AltM' { Key 0x4D -Alt } # Alt context bit 29 set on down AND up.
        'AltSpace' { Key 0x20 -Alt } # Native DefWindowProc/TranslateMessage path; no private open.
        'Caption' {
            $p=[TitleMenuSmoke.Native]::CaptionIcon($window,[uint32]$ownedPid)
            Post $window 0xA1 3 ([TitleMenuSmoke.Native]::Pack($p)) # WM_NCLBUTTONDOWN / verified HTSYSMENU
            Post $window 0xA2 3 ([TitleMenuSmoke.Native]::Pack($p)) # Complete native press/release, no cursor move.
        }
        'MouseMenu' {
            $p=[TitleMenuSmoke.Native]::CaptionIcon($window,[uint32]$ownedPid)
            Post $window 0x112 0xF093 ([TitleMenuSmoke.Native]::Pack($p)) # SC_MOUSEMENU | HTSYSMENU; not a click proof.
        }
        'DoubleCaption' {
            $p=[TitleMenuSmoke.Native]::CaptionIcon($window,[uint32]$ownedPid)
            Post $window 0xA3 3 ([TitleMenuSmoke.Native]::Pack($p)) # interception must open, not SC_CLOSE
        }
        default { throw 'Unsupported menu open method; no fake private-message fallback' }
    }
    Wait-State "$Method visible native #32768 + real menu loop" {
        [TitleMenuSmoke.Native]::InMenu($window,[uint32]$ownedPid) -and @([TitleMenuSmoke.Native]::Popups([uint32]$ownedPid,'#32768')).Count -ge 1
    }
    Check-Availability
    return @([TitleMenuSmoke.Native]::Popups([uint32]$ownedPid,'#32768'))[0]
}
function Cancel-Menu {
    for ($i=0;$i -lt 3;$i++) {
        if (-not [TitleMenuSmoke.Native]::InMenu($window,[uint32]$ownedPid)) { break }
        Key 0x1B
        $timer=[Diagnostics.Stopwatch]::StartNew()
        while ($timer.ElapsedMilliseconds -lt 300 -and [TitleMenuSmoke.Native]::InMenu($window,[uint32]$ownedPid)) { Guard; Start-Sleep -Milliseconds 20 }
    }
    Wait-State 'Escape ended menu loop and destroyed visible menu' {
        -not [TitleMenuSmoke.Native]::InMenu($window,[uint32]$ownedPid) -and @([TitleMenuSmoke.Native]::Popups([uint32]$ownedPid,'#32768')).Count -eq 0
    }
    Assert-NoDialog
}
function Select-Submenu([int]$RootIndex) {
    # TrackPopupMenu starts with no highlighted item. Down (not dialog-style
    # Home) enters native menu selection on Windows; never fabricate WM_COMMAND.
    Key 0x28
    try { Wait-State 'File highlighted by initial Down' { (@(Items $root))[0].Highlighted } }
    catch { Record 'navigation failure root state' @(Items $root); throw }
    for ($i=1;$i -le $RootIndex;$i++) {
        Key 0x28
        $wanted=$i
        Wait-State "root item $wanted highlighted" { (@(Items $root))[$wanted].Highlighted }
    }
    Key 0x27 # Right enters native submenu, not a posted application command.
    Wait-State "submenu $RootIndex visibly tracking" { @([TitleMenuSmoke.Native]::Popups([uint32]$ownedPid,'#32768')).Count -ge 2 }
    return (@(Items $root))[$RootIndex].Submenu
}
function Select-Leaf([IntPtr]$Submenu,[int]$Index) {
    if (-not (@(Items $Submenu))[0].Highlighted) { Key 0x28 }
    Wait-State 'first submenu item highlighted' { (@(Items $Submenu))[0].Highlighted }
    for ($i=1;$i -le $Index;$i++) {
        Key 0x28
        $wanted=$i
        Wait-State "submenu leaf $wanted highlighted" { (@(Items $Submenu))[$wanted].Highlighted }
    }
    $item=(@(Items $Submenu))[$Index]
    if (-not $item.Enabled) { throw 'Attempt to select disabled native menu item' }
    # Hard allowlist: views, About, Exit only. Never directory, import/export, Start/Restart.
    if ($item.Id -notin @(0x3070,0x3080,0x3090,0x30A0,0x30B0,0x3030)) { throw 'Unsafe menu activation denied' }
    Key 0x0D
}
function Page([int]$Index) {
    Open-Menu | Out-Null
    $sub=Select-Submenu 2
    Select-Leaf $sub $Index
    Wait-State "menu selected page $Index, TCM and visible controls" {
        $markers=@(2102,2300,2207,2039)
        if ((Message (Control 2100) 0x130B) -ne $Index -or [TitleMenuSmoke.Native]::InMenu($window,[uint32]$ownedPid)) { return $false }
        for ($j=0;$j -lt 4;$j++) { if ([TitleMenuSmoke.Native]::IsWindowVisible((Control $markers[$j])) -ne ($j -eq $Index)) { return $false } }
        return $true
    }
    Assert-NoDialog
}
function Config-Unchanged {
    Guard
    if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($config)) -cne $configBytes) { throw 'Unexpected config persistence during menu/preview' }
    if ([TitleMenuSmoke.Native]::IsWindowEnabled((Control 2103)) -or [TitleMenuSmoke.Native]::IsWindowEnabled((Control 2003))) { throw 'Proxy unexpectedly running; no network is allowed' }
}
function Sample-Resources([int]$Iteration) {
    Guard
    $sample=[pscustomobject]@{ Iteration=$Iteration; GDI=[TitleMenuSmoke.Native]::Resources($childHandle,[uint32]$ownedPid,0); USER=[TitleMenuSmoke.Native]::Resources($childHandle,[uint32]$ownedPid,1) }
    $resources.Add($sample)
    return $sample
}
function Theme([int]$Index) {
    $theme=Control 2030
    if ([TitleMenuSmoke.Native]::Class($theme) -ne 'SysTabControl32') { throw 'Theme control contract changed' }
    # Theme fixture only: existing compatibility selection + existing notification.
    # This is NOT used as evidence of menu/page/action activation.
    Message $theme 0x14E ([uint64]$Index) | Out-Null # CB_SETCURSEL compatibility
    Post $window 0x111 ([uint64](2030 -bor (1 -shl 16))) ($theme.ToInt64())
    Post $window 0x31A # WM_THEMECHANGED also exercises native refresh/coalescing.
    Wait-State "theme draft $Index selected" { (Message $theme 0x130B) -eq $Index }
    # Cross-thread SendMessage does not fence posted messages. Poll an independent
    # subsequent visible menu; WM_INITMENUPOPUP runs in the actual tracking loop.
    Open-Menu | Out-Null
    Cancel-Menu
}

# Refuse pre-existing PID/mutex BEFORE files, launch, IPC or UI input.
Assert-NoInstance
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Build pending: $exe. No build/GUI run. Use -ValidateOnly until build notification." }
$exe=(Get-Item -LiteralPath $exe).FullName
try {
    $oldDpi=[TitleMenuSmoke.Native]::EnterDpi()
    $tag=[Guid]::NewGuid().ToString('N')
    $temp=Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-title-menu-config-'+$tag)
    $evidenceRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\target\title-menu-screenshots'))
        $output=Join-Path $evidenceRoot ([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')+'-'+$tag)
    [void][IO.Directory]::CreateDirectory($temp)
    [void][IO.Directory]::CreateDirectory($output)
    $config=Join-Path $temp 'config.toml'
    $toml="schema_version = 1`n[upstream]`nurl = 'http://127.0.0.1:1/v1'`n[listen]`naddress = '127.0.0.1'`nport = 4000`n[proxy]`nauto_start = false`nforward_enabled = false`nconnect_enabled = false`n[ui]`ntheme = 'light'`nremember_window_geometry = false`n[system]`nlogin_auto_start = false`n"
    [IO.File]::WriteAllText($config,$toml,[Text.UTF8Encoding]::new($false))
    $configBytes=[Convert]::ToBase64String([IO.File]::ReadAllBytes($config))
    Assert-NoInstance
    $stage='launch and pin isolated stopped child'
    $child=Start-Process -FilePath $exe -WorkingDirectory $temp -ArgumentList @('--config-dir',('"'+$temp+'"')) -PassThru
    $childHandle=$child.Handle; $childStart=$child.StartTime; $ownedPid=$child.Id
    Wait-State 'own main window' { $script:window=[TitleMenuSmoke.Native]::FindMain([uint32]$ownedPid); $window -ne [IntPtr]::Zero } 10000
    Wait-State 'initial asynchronous data/config load complete' {
        $start=[TitleMenuSmoke.Native]::FindControl($window,[uint32]$ownedPid,2102)
        $export=[TitleMenuSmoke.Native]::FindControl($window,[uint32]$ownedPid,2034)
        $status=[TitleMenuSmoke.Native]::FindControl($window,[uint32]$ownedPid,2354)
        $start -ne [IntPtr]::Zero -and $export -ne [IntPtr]::Zero -and $status -ne [IntPtr]::Zero -and
        [TitleMenuSmoke.Native]::IsWindowEnabled($start) -and [TitleMenuSmoke.Native]::IsWindowEnabled($export) -and (Text $status).StartsWith('idle=1;')
    } 10000
    $root=[TitleMenuSmoke.Native]::Root($window,[uint32]$ownedPid)
    $entries=@(Check-Menu)
    Record 'root File/Proxy/View/Help/separator then preserved native SC commands; twelve aligned IDs' $entries
    Record 'pinned child and physical DPI' @{ PID=$ownedPid; DPI=[TitleMenuSmoke.Native]::GetDpiForWindow($window); Exe=$exe }
    Config-Unchanged

    $stage='native caption icon/frame baseline'
    $big=[TitleMenuSmoke.Native]::EffectiveIcon($window,[uint32]$ownedPid,$true) # Window/class fallback, never WM_SETICON.
    $small=[TitleMenuSmoke.Native]::EffectiveIcon($window,[uint32]$ownedPid,$false)
    if ($big -eq [IntPtr]::Zero -or $small -eq [IntPtr]::Zero -or $big -eq $small) { throw 'BIG/SMALL icons missing or not distinct' }
    $bigProof=[TitleMenuSmoke.Native]::IconProof($big,(Join-Path $output 'icon-big.png'))
    $smallProof=[TitleMenuSmoke.Native]::IconProof($small,(Join-Path $output 'icon-small-hamburger.png'))
    $exeIconHash=[TitleMenuSmoke.Native]::ExecutableIcon($exe)
    if ($bigProof.Hash -cne $exeIconHash -or $smallProof.Hash -ceq $bigProof.Hash -or $smallProof.Bands -ne 3) { throw 'BIG differs from executable app icon or SMALL is not a three-band hamburger' }
    $frame=[TitleMenuSmoke.Native]::Frame($window,[uint32]$ownedPid)
    $direct=@([TitleMenuSmoke.Native]::Children($window,[uint32]$ownedPid,$false))
    Record 'direct main HWND census' @($direct | ForEach-Object { [pscustomobject]@{ Handle=$_.ToInt64(); Class=[TitleMenuSmoke.Native]::Class($_); ID=[TitleMenuSmoke.Native]::GetDlgCtrlID($_); Visible=[TitleMenuSmoke.Native]::IsWindowVisible($_) } })
        $visibleDirect=@($direct | Where-Object { [TitleMenuSmoke.Native]::IsWindowVisible($_) })
            # Windows IME may add an invisible OimeTsfNotifyWindow; it is not a strip.
            if ($visibleDirect.Count -ne 3 -or $visibleDirect -notcontains (Control 2100) -or $visibleDirect -notcontains (Control 2400)) { throw 'Unexpected extra visible client strip/main children' }
    $origin=[TitleMenuSmoke.Native]::Origin($window,[uint32]$ownedPid)
    $tabRect=[TitleMenuSmoke.Native]::Outer((Control 2100),[uint32]$ownedPid)
    $hostRect=[TitleMenuSmoke.Native]::Outer((Control 2400),[uint32]$ownedPid)
    $dpi=[TitleMenuSmoke.Native]::GetDpiForWindow($window)
    if ($tabRect.Top-$origin.Y -ne [int][Math]::Round(12*$dpi/96) -or $hostRect.Top-$origin.Y -ne [int][Math]::Round(48*$dpi/96)) { throw 'Tab/viewport shifted by unexpected title/menu client strip' }
    Record 'SMALL hamburger; BIG equals executable resource; native no-menubar frame and fixed children' @{ Big=$bigProof; Small=$smallProof; Frame=$frame }
    Capture 'light-main-native-frame' $window

    $stage='real F10 popup / dynamic stopped availability / Escape'
    $popup=Open-Menu
    Capture 'light-f10-native-menu' $popup -Popup
    Capture 'light-f10-main-window' $window
    Record 'F10 key -> native visible #32768 / availability and page radio checked' $true
    Cancel-Menu
    # The ONE direct custom WM_SYSCOMMAND is disabled STOP. No fake page/action
    # dispatch is allowed. This proves no observable work/modal/config change;
    # zero controller dispatch itself needs in-process instrumentation, not claimed.
    Post $window 0x112 0x3050
    Open-Menu | Out-Null
    Check-Availability
    Cancel-Menu
    Config-Unchanged
    Record 'disabled STOP WM_SYSCOMMAND observational rejection' 'Still stopped/ready, no dialog or config mutation; not a controller-call-count proof'

    $stage='caption native hit-test/click and intercepted double-click'
    foreach ($method in @('F10System','Caption','DoubleCaption','AltM','AltSpace','MouseMenu')) {
        try {
            $popup=Open-Menu $method
            Capture ("light-$method-native-menu") $popup -Popup
            Cancel-Menu
            Guard
            if (-not [TitleMenuSmoke.Native]::IsWindowVisible($window)) { throw "$method unexpectedly hid/closed main window" }
            Record "$method opened visible native menu and Escape cancelled without close" $true
        } catch {
            $reason=$_.Exception.GetBaseException().Message
            if ($method -eq 'MouseMenu') {
                Record 'DIAGNOSTIC ONLY native SC_MOUSEMENU (not a supported application entry)' $reason
            } else {
                $uiFailures.Add("${method}: $reason")
                Record "FAIL $method; continuing independent evidence" $reason
            }
            Guard
            if ([TitleMenuSmoke.Native]::InMenu($window,[uint32]$ownedPid)) { Cancel-Menu }
            Assert-NoDialog
        }
    }

    $stage='actual menu View keyboard selection / draft and per-page offsets'
    Page 1
    $historyEdit=Control 2304
    $historyDraft='127.0.0.2'
    [TitleMenuSmoke.Native]::SetText($historyEdit,[uint32]$ownedPid,$historyDraft)
    Page 3
    $edits=@([TitleMenuSmoke.Native]::Children($window,[uint32]$ownedPid,$true) | Where-Object {
        [TitleMenuSmoke.Native]::Class($_) -eq 'Edit' -and [TitleMenuSmoke.Native]::GetDlgCtrlID($_) -eq 0
    })
    # Text() refuses ES_PASSWORD. Filter by the API's refusal rather than ever
    # disabling password protection or copying any non-fixture input.
    $urlEdits=[Collections.Generic.List[IntPtr]]::new()
    foreach ($h in $edits) {
        try { $value=Text $h } catch { if ($_.Exception.GetBaseException().Message -eq 'Password read denied') { continue }; throw }
        if ($value -ceq 'http://127.0.0.1:1/v1') { $urlEdits.Add($h) }
    }
    if ($urlEdits.Count -ne 1) { throw 'Cannot uniquely identify synthetic settings URL Edit' }
    $draftEdit=$urlEdits[0]; $draft='http://127.0.0.1:1/title-menu-draft'
    [TitleMenuSmoke.Native]::SetText($draftEdit,[uint32]$ownedPid,$draft)
    $offsets=@{}
    foreach ($pageIndex in 0..3) {
        Page $pageIndex
        $hostWindow=Control 2400
        Message $hostWindow 0x115 6 | Out-Null # SB_TOP
        for ($line=0;$line -le $pageIndex;$line++) { Message $hostWindow 0x115 1 | Out-Null }
        $offsets[[string]$pageIndex]=[TitleMenuSmoke.Native]::Offset($hostWindow,[uint32]$ownedPid)
    }
    if (@($offsets.Values | Where-Object { $_ -gt 0 }).Count -ne 4) { throw 'No meaningful nonzero offset fixture on all pages' }
    foreach ($pageIndex in @(3,1,0,2,3,1)) {
        Page $pageIndex
        if ([TitleMenuSmoke.Native]::Offset((Control 2400),[uint32]$ownedPid) -ne $offsets[[string]$pageIndex]) { throw "Page $pageIndex lost independent scroll offset" }
        if ((Text $draftEdit) -cne $draft -or (Text $historyEdit) -cne $historyDraft) { throw 'Menu page switch discarded unsaved draft' }
    }
    $popup=Open-Menu
    $view=Select-Submenu 2
    $submenuPopup=@([TitleMenuSmoke.Native]::Popups([uint32]$ownedPid,'#32768') | Where-Object { $_ -ne $popup })
    if ($submenuPopup.Count -ne 1) { throw 'Cannot uniquely identify visible View submenu' }
    Capture 'view-submenu-native-keyboard' $submenuPopup[0] -Popup
    Cancel-Menu
    Config-Unchanged
    Record 'native Down/Right/Enter View navigation, TCM selection+page visibility and drafts/offsets retained' $offsets

    $stage='native Help About selection and verified sole own button'
    Open-Menu | Out-Null
    $help=Select-Submenu 3
    Select-Leaf $help 0
    Wait-State 'own About MessageBox' { @([TitleMenuSmoke.Native]::Popups([uint32]$ownedPid,'#32770')).Count -eq 1 }
    $about=@([TitleMenuSmoke.Native]::Popups([uint32]$ownedPid,'#32770'))[0]
    if ((Text $about) -cne '关于 AI Relay' -or [TitleMenuSmoke.Native]::GetWindow($about,4) -ne $window) { throw 'Unexpected modal; not acknowledged' }
    Capture 'about-own-native-messagebox' $about
    $aboutButtons=@([TitleMenuSmoke.Native]::Children($about,[uint32]$ownedPid,$true) | Where-Object {
        [TitleMenuSmoke.Native]::Class($_) -eq 'Button' -and
        [TitleMenuSmoke.Native]::IsWindowVisible($_) -and [TitleMenuSmoke.Native]::IsWindowEnabled($_)
    })
    if ($aboutButtons.Count -ne 1) { throw 'About must have exactly one visible enabled Button; not acknowledged' }
    $ok=$aboutButtons[0]
    $aboutButtonId=[TitleMenuSmoke.Native]::GetDlgCtrlID($ok)
    if ($aboutButtonId -notin @(1,2) -or [TitleMenuSmoke.Native]::GetParent($ok) -ne $about) { throw 'Unexpected About button ID/parent; not acknowledged' }
    Record 'About verified sole visible enabled native Button' @{ ID=$aboutButtonId; Text=(Text $ok); OwnerPID=$ownedPid }
    Post $ok 0xF5 # Actual BM_CLICK of the verified IDOK/IDCANCEL control, never fake dialog success.
    Wait-State 'About dismissed by actual native button' { @([TitleMenuSmoke.Native]::Popups([uint32]$ownedPid,'#32770')).Count -eq 0 }
    Record 'Help native selection -> own About -> actual sole ID 1/2 Button click' $true

    $stage='preserved OS maximize/restore (never minimize/close)'
    $beforeOuter=[TitleMenuSmoke.Native]::Outer($window,[uint32]$ownedPid).ToString()
    $beforeClient=[TitleMenuSmoke.Native]::Client($window,[uint32]$ownedPid).ToString()
    if ([TitleMenuSmoke.Native]::IsZoomed($window)) { throw 'Unexpected maximized initial fixture' }
    Post $window 0x112 0xF030
    Wait-State 'native IsZoomed after SC_MAXIMIZE' { [TitleMenuSmoke.Native]::IsZoomed($window) }
    Post $window 0x112 0xF120
    Wait-State 'native restore and original physical client/outer geometry' {
        -not [TitleMenuSmoke.Native]::IsZoomed($window) -and [TitleMenuSmoke.Native]::Outer($window,[uint32]$ownedPid).ToString() -ceq $beforeOuter -and
        [TitleMenuSmoke.Native]::Client($window,[uint32]$ownedPid).ToString() -ceq $beforeClient
    }
    [TitleMenuSmoke.Native]::Frame($window,[uint32]$ownedPid) | Out-Null
    Record 'SC_MAXIMIZE/SC_RESTORE delegated to native window manager; original client geometry restored' $true

    $stage='light/dark native menu screenshots and 30 bounded theme refreshes'
    $baseline=Sample-Resources 0
    foreach ($themeIndex in @(2,1)) {
        Theme $themeIndex
        $name=if ($themeIndex -eq 2) { 'dark' } else { 'light' }
        $popup=Open-Menu
        Capture "$name-native-popup-system-colors-allowed" $popup -Popup
        Capture "$name-main-caption" $window
        Cancel-Menu
        $bigNow=[TitleMenuSmoke.Native]::EffectiveIcon($window,[uint32]$ownedPid,$true)
        $smallNow=[TitleMenuSmoke.Native]::EffectiveIcon($window,[uint32]$ownedPid,$false)
        $smallNowProof=[TitleMenuSmoke.Native]::IconProof($smallNow,(Join-Path $output "$name-hamburger.png"))
        if ($bigNow -ne $big -or $smallNow -eq $big -or $smallNowProof.Bands -ne 3) { throw 'Theme replaced BIG or invalidated hamburger' }
        Config-Unchanged
    }
    for ($iteration=1;$iteration -le 30;$iteration++) {
        Theme (1+($iteration%2))
        $sample=Sample-Resources $iteration
        if ($sample.GDI -gt $baseline.GDI+24 -or $sample.USER -gt $baseline.USER+16) { throw "Unbounded GDI/USER growth at refresh $iteration" }
        if ([TitleMenuSmoke.Native]::EffectiveIcon($window,[uint32]$ownedPid,$true) -ne $big) { throw 'BIG icon handle changed during theme refresh' }
    }
    $final=Sample-Resources 31
    if ($final.GDI -gt $baseline.GDI+12 -or $final.USER -gt $baseline.USER+8) { throw 'GDI/USER did not settle within bounded tolerance' }
    if ((Text $draftEdit) -cne $draft -or (Text $historyEdit) -cne $historyDraft) { throw 'Theme/menu refresh lost drafts' }
    Config-Unchanged
    Record '30 alternating theme+WM_THEMECHANGED refreshes; native menus may keep system palette' $resources.ToArray()

    $stage='actual File Exit menu selection and normal asynchronous flush/exit'
    # No Save/Reload/import/export, clipboard, startup, proxy start, or file dialog.
    Open-Menu | Out-Null
    $file=Select-Submenu 0
    Select-Leaf $file 3
    $exitTimer=[Diagnostics.Stopwatch]::StartNew()
    while (-not $child.HasExited -and $exitTimer.ElapsedMilliseconds -lt 15000) {
        Assert-NoOtherProcess
        if ($clock.Elapsed.TotalSeconds -ge $BudgetSeconds) { throw 'Overall budget expired waiting for graceful menu Exit' }
        # The pinned Process waits on its original handle. A CheckProcess(live)
        # here races the very exit being tested and can falsely report failure.
        if ($child.WaitForExit(30)) { break }
        $child.Refresh()
    }
    if (-not $child.HasExited) { throw 'File/Exit did not exit within 15s; graceful flush not proven' }
    if ($child.ExitCode -ne 0) { throw "Owned child failed: exit=$($child.ExitCode)" }
    $normalExit=$true
    Record 'File Exit selected through native keys, process exited normally after app flush path' @{ ExitCode=$child.ExitCode; Milliseconds=$exitTimer.ElapsedMilliseconds }
} catch {
    $failure=$_
    Write-Warning ("FAIL at {0}: {1}" -f $stage,$_.Exception.GetBaseException().Message)
} finally {
    # Cleanup can kill ONLY the original pinned process. No CLI quit/IPC broadcast,
    # no foreign-instance messages, and no deletion while the child may own files.
    if ($null -ne $child -and -not $child.HasExited) {
        try {
            Assert-Identity
            $child.Kill()
            if (-not $child.WaitForExit(5000)) { throw 'Owned child cleanup exceeded 5s' }
        } catch { $cleanupErrors.Add($_.Exception.GetBaseException().Message) }
    }
    if ($oldDpi -ne [IntPtr]::Zero) { [TitleMenuSmoke.Native]::SetThreadDpiAwarenessContext($oldDpi) | Out-Null }
    if ($null -ne $temp -and (Test-Path -LiteralPath $temp)) {
        if ($null -eq $child -or $child.HasExited) {
            try { Remove-Item -LiteralPath $temp -Recurse -Force } catch { $cleanupErrors.Add($_.Exception.Message) }
        } else { $cleanupErrors.Add("Child still live; temporary config retained: $temp") }
    }
    if ($null -ne $output) {
        $report=[ordered]@{
            Complete=$false # 仍须目视截图和报告中的人工验收项。
            AutomatedScopeComplete=($null -eq $failure -and $normalExit -and $cleanupErrors.Count -eq 0 -and $uiFailures.Count -eq 0)
            Stage=$stage; Failure=$(if ($null -eq $failure) { $null } else { $failure.Exception.GetBaseException().Message })
            Exe=$exe; PID=$ownedPid; Seconds=[Math]::Round($clock.Elapsed.TotalSeconds,2)
            Checks=$checks.ToArray(); Screenshots=$shots.ToArray(); Resources=$resources.ToArray(); CleanupErrors=$cleanupErrors.ToArray(); UiFailures=$uiFailures.ToArray()
            Limits=@(
                'HWND-local keyboard/NC messages drive the actual native menu loop; not physical hardware/SendInput or global-pointer coverage.',
                'F10 WM_KEYDOWN/WM_SYSKEYDOWN pairs (context clear), Alt+M and Alt+Space (context bit 29 set), verified HTSYSMENU click and double-click are HWND-local events, not physical chords.',
                'Pre-attachment geometry is not observable externally. Runtime comparison uses native no-menu frame metrics, existing Tab/viewport positions and three visible main children (hidden Windows IME helpers allowed).',
                'BIG is compared with executable icon and retained across themes; shell taskbar/Alt+Tab/tray visual identity still needs manual acceptance.',
                'Initial async config/data load completes before open; menu-loop queries check responsiveness. No deliberate data-reload-under-menu race or Rust borrow-safety proof.',
                'Disabled STOP is observational only; no dispatch counter. Directory/import/export/Start/Restart/Save/Reload/Startup/clipboard commands are never activated; File Exit is exercised.',
                'SC_MOUSEMENU remains a native diagnostic, not a supported application entry or acceptance failure. WM_NCLBUTTONDOWN HTSYSMENU must independently open a visible native menu.',
                'Menu/MessageBox colors may remain system colors in dark mode. Screenshots need visual review; no forced-dark assertion.',
                'Only current physical DPI/monitor and 30 refreshes; no multi-monitor, high-contrast switching, reader or long-soak acceptance.',
                'Normal exit verifies the existing asynchronous flush path completed with exit code zero, not crash/power-loss durability.'
            )
        }
        try { [IO.File]::WriteAllText((Join-Path $output 'report.json'),($report | ConvertTo-Json -Depth 10),[Text.UTF8Encoding]::new($false)) }
        catch { $cleanupErrors.Add("Report write failed: $($_.Exception.Message)") }
        Write-Host "Evidence/report: $output"
    }
    if ($null -ne $child) { $child.Dispose() }
}
if ($null -ne $failure) { throw $failure }
if ($cleanupErrors.Count -gt 0) { throw ($cleanupErrors -join '; ') }
if ($uiFailures.Count -gt 0) { throw ($uiFailures -join '; ') }
Write-Output 'PASS: title-menu automated scope completed; inspect retained screenshots and report Limits for remaining manual acceptance'
