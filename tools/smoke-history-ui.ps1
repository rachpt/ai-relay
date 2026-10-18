# Local mock, synthetic credentials, own child/temp data only. Never touches Startup.
[CmdletBinding()]
param(
    [string]$ExePath,
    [ValidateRange(5,30)][int]$StepTimeoutSeconds = 20,
    # Explicit opt-in: bounded text-only clipboard snapshot, sequence/owner guards.
    [switch]$VerifyCellClipboard
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([IntPtr]::Size -ne 8) { throw 'Use 64-bit powershell.exe -NoProfile' }
# A different config directory does NOT bypass the user/session singleton.
# Check only; never send quit/stop to, or kill, a pre-existing main instance.
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$session = [Diagnostics.Process]::GetCurrentProcess().SessionId
$instanceName = "Local\AIRelay.$sid.$session.v1"
try { $existing = [Threading.Mutex]::OpenExisting($instanceName) } catch [Threading.WaitHandleCannotBeOpenedException] { $existing = $null }
if ($null -ne $existing) { $existing.Dispose(); throw 'Existing application instance: history GUI smoke skipped to protect user state.' }
if (-not $ExePath) { $ExePath = Join-Path $PSScriptRoot '..\target\debug\ai-relay.exe' }
$exe = (Get-Item -LiteralPath $ExePath).FullName
$source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'smoke-proxy-ui.ps1'))
$match = [regex]::Match($source, "(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@")
if (-not $match.Success) { throw 'Missing PID-checked smoke helpers' }
$proxyDeclaration = $match.Groups[1].Value
$historyDeclaration = @'
using System;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading.Tasks;
public static class HistoryUi {
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    public static void CheckOwner(IntPtr h,uint expected) {
        uint pid; GetWindowThreadProcessId(h,out pid);
        if(h==IntPtr.Zero || pid!=expected) throw new Exception("Refusing to access another process or missing history control");
    }
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr h, uint m, UIntPtr w, IntPtr l, uint f, uint t, out UIntPtr result);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr h, uint m, UIntPtr w, StringBuilder l, uint f, uint t, out UIntPtr result);
    public static int Value(IntPtr h, uint pid, uint message) {
        CheckOwner(h,pid); UIntPtr result;
        if(SendMessageTimeoutW(h,message,UIntPtr.Zero,IntPtr.Zero,2,2000,out result)==IntPtr.Zero) throw new Exception("Scalar control query failed");
        return unchecked((int)result.ToUInt64());
    }
    public static string Text(IntPtr h, uint pid) {
        CheckOwner(h,pid); UIntPtr result; var text=new StringBuilder(262144);
        if(SendMessageTimeoutW(h,13,new UIntPtr((uint)text.Capacity),text,2,2000,out result)==IntPtr.Zero) throw new Exception("Text read failed");
        return text.ToString();
    }
}
public sealed class HistoryMock : IDisposable {
    const string UsageJson = "{\"model\":\"local-usage-model\",\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}";
    readonly TcpListener listener;
    readonly Task task;
    volatile bool stopping;
    public int Port { get; private set; }
    public HistoryMock() {
        listener=new TcpListener(IPAddress.Loopback,0); listener.Start();
        Port=((IPEndPoint)listener.LocalEndpoint).Port;
        task=Task.Run(async delegate {
            try {
                while(!stopping) {
                    using(var client=await listener.AcceptTcpClientAsync()) {
                        client.ReceiveTimeout=3000; client.SendTimeout=3000;
                        using(var stream=client.GetStream())
                        using(var reader=new StreamReader(stream,Encoding.UTF8,false,1024,true)) {
                            string first=reader.ReadLine(), line; int length=0, headers=0;
                            while(!String.IsNullOrEmpty(line=reader.ReadLine())) {
                                headers+=line.Length; if(headers>16384) throw new Exception("Mock header limit");
                                if(line.StartsWith("Content-Length:",StringComparison.OrdinalIgnoreCase)) length=int.Parse(line.Substring(15).Trim());
                            }
                            if(length<0 || length>4096) throw new Exception("Mock body limit");
                            var body=new char[length]; int read=0;
                            while(read<length) { int n=reader.Read(body,read,length-read); if(n==0) throw new Exception("Short mock body"); read+=n; }
                            bool valid=first=="GET /v1/usage-json HTTP/1.1" || first=="POST /v1/chat/completions HTTP/1.1";
                            string response=valid?UsageJson:"bad mock target";
                            byte[] bytes=Encoding.UTF8.GetBytes("HTTP/1.1 "+(valid?"200 OK":"400 Bad Request")+"\r\nContent-Type: application/json\r\nContent-Length: "+Encoding.UTF8.GetByteCount(response)+"\r\nConnection: close\r\n\r\n"+response);
                            stream.Write(bytes,0,bytes.Length);
                        }
                    }
                }
            } catch(ObjectDisposedException) { if(!stopping) throw; }
              catch(SocketException) { if(!stopping) throw; }
        });
    }
    public void Dispose() { stopping=true; listener.Stop(); if(!task.Wait(5000)) throw new Exception("Mock shutdown timeout"); }
}
public static class HistoryRequest {
    public static void Get(int port, string token) { Send(port,token,false); }
    public static void Post(int port, string token) { Send(port,token,true); }
    static void Send(int port, string token, bool post) {
        var r=(HttpWebRequest)WebRequest.Create("http://127.0.0.1:"+port+(post?"/chat/completions":"/usage-json"));
        r.Proxy=null; r.Timeout=3000; r.ReadWriteTimeout=3000;
        r.Headers["Authorization"]="Bearer "+token;
        if(post) {
            r.Method="POST"; r.ContentType="application/json";
            byte[] data=Encoding.UTF8.GetBytes("{\"model\":\"local-requested-model\",\"messages\":[]}");
            r.ContentLength=data.Length;
            using(var body=r.GetRequestStream()) body.Write(data,0,data.Length);
        }
        using(var response=r.GetResponse()) using(var reader=new StreamReader(response.GetResponseStream())) {
            if(!reader.ReadToEnd().Contains("local-usage-model")) throw new Exception("Mock response mismatch");
        }
    }
}
'@
# Declarations only; no other smoke script is executed. WM_USER+ structures must
# live in the pinned child's VM, never in this PowerShell process's address space.
$cellDeclaration = @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
public sealed class HistoryCells : IDisposable {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
    [StructLayout(LayoutKind.Sequential)] struct Item {
        public uint Mask; public int Row,Column; public uint State,StateMask;
        public IntPtr Text; public int Capacity,Image; public IntPtr Param;
        public int Indent,GroupId; public uint Columns; public IntPtr ColumnIndices,ColumnFormats; public int Group;
    }
    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access,bool inherit,uint pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")] static extern uint GetProcessId(IntPtr h);
    [DllImport("kernel32.dll")] static extern bool GetExitCodeProcess(IntPtr h,out uint code);
    [DllImport("kernel32.dll")] static extern IntPtr VirtualAllocEx(IntPtr p,IntPtr a,UIntPtr size,uint kind,uint protect);
    [DllImport("kernel32.dll")] static extern bool VirtualFreeEx(IntPtr p,IntPtr a,UIntPtr size,uint kind);
    [DllImport("kernel32.dll")] static extern bool WriteProcessMemory(IntPtr p,IntPtr a,byte[] b,UIntPtr size,out UIntPtr count);
    [DllImport("kernel32.dll")] static extern bool ReadProcessMemory(IntPtr p,IntPtr a,byte[] b,UIntPtr size,out UIntPtr count);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr h,uint m,UIntPtr w,IntPtr l,uint flags,uint ms,out UIntPtr result);
    [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr h,uint m,UIntPtr w,IntPtr l);
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h,out Rect r);
    [DllImport("user32.dll")] static extern int MapWindowPoints(IntPtr from,IntPtr to,ref Rect r,uint count);

    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h,int command);
    [DllImport("user32.dll")] static extern bool EnumWindows(ProxyUiSmoke.EnumProc callback,IntPtr p);
    [DllImport("user32.dll")] static extern uint GetMenuState(IntPtr menu,uint id,uint flags);
    [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr menu);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetMenuStringW(IntPtr menu,uint id,StringBuilder s,int size,uint flags);
    readonly IntPtr pinned,process;
    public readonly IntPtr Table;
    public readonly uint Pid;
    static readonly int ItemSize=Marshal.SizeOf(typeof(Item));
    public HistoryCells(IntPtr pinned,uint pid,IntPtr table) {
        this.pinned=pinned; Pid=pid; Table=table;
        CheckProcess(pinned,pid); HistoryUi.CheckOwner(table,pid);
        if(IntPtr.Size!=8 || ItemSize!=88 || Marshal.OffsetOf(typeof(Item),"Text").ToInt32()!=24)
            throw new Exception("Unexpected x64 LVITEMW layout");
        if(ProxyUiSmoke.Class(table)!="SysListView32") throw new Exception("Expected native report table");
        process=OpenProcess(0x101038,false,pid);
        if(process==IntPtr.Zero) throw new Exception("Cannot open owned child VM");
        try { CheckProcess(process,pid); } catch { CloseHandle(process); throw; }
    }
    static void CheckProcess(IntPtr h,uint pid) {
        uint code;
        if(h==IntPtr.Zero || GetProcessId(h)!=pid || !GetExitCodeProcess(h,out code) || code!=259)
            throw new Exception("Pinned history child identity/liveness changed");
    }
    void Check(IntPtr h) { CheckProcess(pinned,Pid); CheckProcess(process,Pid); HistoryUi.CheckOwner(h,Pid); }
    public long Message(uint m,long w,long l) { Check(Table); return Send(Table,Pid,m,w,l); }
    public static long Send(IntPtr h,uint pid,uint m,long w,long l) {
        HistoryUi.CheckOwner(h,pid); UIntPtr result;
        if(SendMessageTimeoutW(h,m,new UIntPtr(unchecked((ulong)w)),new IntPtr(l),0x22,2000,out result)==IntPtr.Zero)
            throw new Exception("History cell message timeout/failure: 0x"+m.ToString("X"));
        return unchecked((long)result.ToUInt64());
    }
    public static void Post(IntPtr h,uint pid,uint m,long w,long l) {
        HistoryUi.CheckOwner(h,pid);
        if(!PostMessageW(h,m,new UIntPtr(unchecked((ulong)w)),new IntPtr(l))) throw new Exception("History cell PostMessage failed");
    }
    static byte[] Bytes(Item item,int textBytes) {
        var bytes=new byte[ItemSize+textBytes]; IntPtr local=Marshal.AllocHGlobal(ItemSize);
        try { Marshal.StructureToPtr(item,local,false); Marshal.Copy(local,bytes,0,ItemSize); }
        finally { Marshal.FreeHGlobal(local); }
        return bytes;
    }
    byte[] Native(IntPtr h,uint m,long w,byte[] bytes,bool text,out long result) {
        Check(h);
        if(bytes.Length<16 || bytes.Length>2097248) throw new Exception("Native cell buffer exceeds bound");
        IntPtr remote=VirtualAllocEx(process,IntPtr.Zero,new UIntPtr((uint)bytes.Length),0x3000,4);
        if(remote==IntPtr.Zero) throw new Exception("Native cell allocation failed");
        bool complete=true;
        try {
            if(text) Array.Copy(BitConverter.GetBytes(IntPtr.Add(remote,ItemSize).ToInt64()),0,bytes,24,8);
            UIntPtr count,value;
            if(!WriteProcessMemory(process,remote,bytes,new UIntPtr((uint)bytes.Length),out count) || count.ToUInt64()!=(ulong)bytes.Length)
                throw new Exception("Native cell initialization incomplete");
            Check(h); complete=false;
            if(SendMessageTimeoutW(h,m,new UIntPtr(unchecked((ulong)w)),remote,0x22,2000,out value)==IntPtr.Zero)
                throw new Exception("Native cell call timed out; remote buffer retained until own child exits");
            complete=true; result=unchecked((long)value.ToUInt64());
            if(!ReadProcessMemory(process,remote,bytes,new UIntPtr((uint)bytes.Length),out count) || count.ToUInt64()!=(ulong)bytes.Length)
                throw new Exception("Native cell read incomplete");
            return bytes;
        } finally {
            bool released=!complete || VirtualFreeEx(process,remote,UIntPtr.Zero,0x8000);
            if(!released) throw new Exception("Native cell allocation release failed");
        }
    }
    public string Cell(int row,int column) {
        for(int capacity=256;capacity<=1048578;capacity=Math.Min(capacity*2,1048578)) {
            long length; var bytes=Native(Table,0x1073,row,Bytes(new Item { Column=column,Capacity=capacity },capacity*2),true,out length);
            if(length<0 || length>=capacity || BitConverter.ToUInt16(bytes,ItemSize+(int)length*2)!=0) throw new Exception("Invalid cell text response");
            if(length<capacity-1) return new UnicodeEncoding(false,false,true).GetString(bytes,ItemSize,(int)length*2);
            if(capacity==1048578) break;
        }
        throw new Exception("Cell text exceeds smoke bound");
    }
    public void SetCell(int row,int column,string text) {
        if(text.Length>32768 || text.IndexOf('\0')>=0) throw new Exception("Invalid local Unicode fixture");
        byte[] value=Encoding.Unicode.GetBytes(text+"\0"),bytes=Bytes(new Item { Column=column },value.Length);
        Array.Copy(value,0,bytes,ItemSize,value.Length); long result;
        Native(Table,0x1074,row,bytes,true,out result);
        if(result==0) throw new Exception("LVM_SETITEMTEXTW fixture failed");
    }
    public void SelectRow(int row) {
        long result;
        Native(Table,0x102B,-1,Bytes(new Item { StateMask=3 },0),false,out result);
        Native(Table,0x102B,row,Bytes(new Item { StateMask=3,State=3 },0),false,out result);
        if(Selected()!=row) throw new Exception("Native row selection mismatch");
    }
    public int Selected() { return (int)Message(0x100C,-1,3); }
    Rect NativeRect(IntPtr h,uint m,int index,int left,int top) {
        var bytes=new byte[16]; Array.Copy(BitConverter.GetBytes(left),0,bytes,0,4); Array.Copy(BitConverter.GetBytes(top),0,bytes,4,4);
        long result; bytes=Native(h,m,index,bytes,false,out result);
        if(result==0) throw new Exception("Native cell/header RECT failed");
        return new Rect { Left=BitConverter.ToInt32(bytes,0),Top=BitConverter.ToInt32(bytes,4),Right=BitConverter.ToInt32(bytes,8),Bottom=BitConverter.ToInt32(bytes,12) };
    }
    public Rect CellRect(int row,int column) {
        var rect=NativeRect(Table,0x1038,row,0,column);
        IntPtr header=new IntPtr(Message(0x101F,0,0)); Check(header);
        var horizontal=NativeRect(header,0x1207,column,0,0);
        MapWindowPoints(header,Table,ref horizontal,2); rect.Left=horizontal.Left; rect.Right=horizontal.Right;
        return rect;
    }
    public Point Reveal(int row,int column) {
        Message(0x1013,row,0); var rect=CellRect(row,column); Rect client;
        if(!GetClientRect(Table,out client)) throw new Exception("Table client RECT failed");
        int dx=rect.Left<0?rect.Left:rect.Right>client.Right?Math.Min(rect.Left,rect.Right-client.Right):0;
        if(dx!=0) Message(0x1014,dx,0);
        rect=CellRect(row,column);
        var p=new Point { X=(Math.Max(rect.Left,0)+Math.Min(rect.Right,client.Right))/2,Y=(rect.Top+rect.Bottom)/2 };
        if(p.X<0 || p.Y<0 || p.X>=client.Right || p.Y>=client.Bottom || rect.Right<=rect.Left)
            throw new Exception("Cell not revealed inside table client");
        return p;
    }
    static long Packed(Point p) { return unchecked((int)(((uint)p.Y<<16)|((uint)p.X&65535))); }
    public void ClickCell(int row,int column) { Click(Reveal(row,column)); }
    void Click(Point p) {
        Check(Table);
        // Queue the entire local sequence; native DOWN tracking must see queued UP.
        Post(Table,Pid,0x201,1,Packed(p)); Post(Table,Pid,0x202,0,Packed(p));
    }
    public void ClickBlank() {
        int count=(int)Message(0x1004,0,0); Rect client;
        if(count<1 || !GetClientRect(Table,out client)) throw new Exception("Missing blank-cell fixture");
        Message(0x1013,count-1,0);
        var last=CellRect(count-1,0);
        if(last.Bottom+2>=client.Bottom) throw new Exception("No blank client area below native rows");
        Click(new Point { X=4,Y=last.Bottom+1 });
    }
    public void Visible(bool visible) { Check(Table); ShowWindow(Table,visible?8:0); }
    static IntPtr Popup(uint pid) {
        IntPtr found=IntPtr.Zero;
        EnumWindows(delegate(IntPtr h,IntPtr p) {
            if(ProxyUiSmoke.Owner(h)==pid && ProxyUiSmoke.Class(h)=="#32768" && ProxyUiSmoke.IsWindowVisible(h)) found=h;
            return true;
        },IntPtr.Zero);
        return found;
    }
    public string Menu(bool keyboard,int row,int column,bool enabled) {
        Check(Table);
        if(Popup(Pid)!=IntPtr.Zero) throw new Exception("Unexpected pre-existing child popup");
        if(keyboard) Post(Table,Pid,0x7B,Table.ToInt64(),-1);
        else {
            var point=Reveal(row,column);
            // True HWND-local queued right-button path, not a global mouse event.
            Post(Table,Pid,0x204,2,Packed(point)); Post(Table,Pid,0x205,0,Packed(point));
        }
        try {
            var timer=Stopwatch.StartNew(); IntPtr popup=IntPtr.Zero;
            while(timer.ElapsedMilliseconds<2000 && (popup=Popup(Pid))==IntPtr.Zero) Thread.Sleep(20);
            if(popup==IntPtr.Zero) throw new Exception("Native cell popup did not appear");
            IntPtr menu=new IntPtr(Send(popup,Pid,0x1E1,0,0));
            var caption=new StringBuilder(64);
            if(menu==IntPtr.Zero || GetMenuItemCount(menu)!=1 || GetMenuStringW(menu,0,caption,64,0x400)==0 || caption.ToString()!="复制单元格\tCtrl+C")
                throw new Exception("Native cell popup caption/count mismatch");
            uint state=GetMenuState(menu,0,0x400);
            if(state==UInt32.MaxValue || ((state&3)==0)!=enabled) throw new Exception("Native cell popup enablement mismatch");
            return "PASS: native "+(keyboard?"keyboard":"right-button")+" cell menu appeared, "+(enabled?"enabled":"disabled")+", cancelled; menu command not invoked";
        } finally {
            // Send cancellation directly: a queued WM_CANCELMODE is not required
            // to be dispatched by TrackPopupMenu's private message loop.
            Message(0x1F,0,0);
            var timer=Stopwatch.StartNew();
            while(timer.ElapsedMilliseconds<2000 && Popup(Pid)!=IntPtr.Zero) Thread.Sleep(20);
            if(Popup(Pid)!=IntPtr.Zero) throw new Exception("Owned cell menu cancellation timed out");
            Message(0,0,0); // bounded acknowledgement after cancellation unwinds
        }
    }
    public void Dispose() { CloseHandle(process); }
}

// Opt-in only. Preserve raw bounded text formats under the clipboard lock; no
// OLE deserialization. CF_LOCALE is allowed only as a 4-byte text companion.
public sealed class HistoryCellClipboard : IDisposable {
    const int Bound=2097152;
    [DllImport("user32.dll")] static extern bool OpenClipboard(IntPtr h);
    [DllImport("user32.dll")] static extern bool CloseClipboard();
    [DllImport("user32.dll")] static extern bool EmptyClipboard();
    [DllImport("user32.dll",SetLastError=true)] static extern uint EnumClipboardFormats(uint f);
    [DllImport("kernel32.dll",EntryPoint="SetLastError")] static extern void ClearError(uint error);
    [DllImport("user32.dll")] static extern IntPtr GetClipboardData(uint f);
    [DllImport("user32.dll")] static extern IntPtr SetClipboardData(uint f,IntPtr data);
    [DllImport("user32.dll")] static extern uint GetClipboardSequenceNumber();
    [DllImport("user32.dll")] static extern IntPtr GetClipboardOwner();
    [DllImport("kernel32.dll")] static extern UIntPtr GlobalSize(IntPtr h);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalLock(IntPtr h);
    [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr h);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalAlloc(uint flags,UIntPtr size);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalFree(IntPtr h);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr CreateWindowExW(uint ex,string cls,string title,uint style,int x,int y,int w,int h,IntPtr parent,IntPtr menu,IntPtr instance,IntPtr data);
    [DllImport("user32.dll")] static extern bool DestroyWindow(IntPtr h);
    readonly Dictionary<uint,byte[]> saved=new Dictionary<uint,byte[]>();
    readonly IntPtr local;
    uint sequence; IntPtr owner; bool changed,disposed;
    public bool CanRun { get; private set; }
    public string Status { get; private set; }

    public HistoryCellClipboard() {
        local=CreateWindowExW(0,"STATIC","history-cell-clipboard-guard",0x80000000,0,0,0,0,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero);
        if(local==IntPtr.Zero) throw new Exception("Cannot create own hidden clipboard guard");
        try {
            Lock();
            try {
                sequence=GetClipboardSequenceNumber(); owner=GetClipboardOwner(); int total=0; uint format=0;
                var formats=new List<uint>();
                // Inspect the complete format set BEFORE reading/rendering any
                // text; mixed image/file/custom clipboards are left untouched.
                while(true) {
                    ClearError(0); format=EnumClipboardFormats(format);
                    if(format==0) {
                        if(Marshal.GetLastWin32Error()!=0) throw new Exception("Cannot enumerate clipboard; no copy attempted");
                        break;
                    }
                    if(format!=1 && format!=7 && format!=13 && format!=16) { Status="NOT RUN: non-text clipboard retained unchanged"; return; }
                    if(formats.Count>=4) { Status="NOT RUN: clipboard format bound exceeded; retained unchanged"; return; }
                    formats.Add(format);
                }
                foreach(uint current in formats) {
                    format=current;
                    IntPtr data=GetClipboardData(format); ulong size=GlobalSize(data).ToUInt64();
                    if(data==IntPtr.Zero || size==0 || size>(ulong)(Bound-total) || (format==16 && size!=4)) {
                        Status="NOT RUN: oversized/unavailable clipboard retained unchanged"; return;
                    }
                    byte[] bytes=Read(data,(int)size);
                    if(format!=16 && !Terminated(format,bytes)) { Status="NOT RUN: malformed text clipboard retained unchanged"; return; }
                    saved.Add(format,bytes); total+=bytes.Length;
                }
                if(saved.ContainsKey(16) && saved.Count==1) { Status="NOT RUN: standalone clipboard locale retained unchanged"; return; }
                Guard(); CanRun=true; Status="READY: bounded text-only/empty clipboard snapshot";
            } finally { CloseClipboard(); }
        } catch { DestroyWindow(local); throw; }
    }
    static bool Terminated(uint format,byte[] bytes) {
        if(format==13) {
            if(bytes.Length%2!=0) return false;
            for(int i=0;i<bytes.Length;i+=2) if(bytes[i]==0 && bytes[i+1]==0) return true;
            return false;
        }
        return Array.IndexOf(bytes,(byte)0)>=0;
    }
    static byte[] Read(IntPtr h,int size) {
        IntPtr p=GlobalLock(h); if(p==IntPtr.Zero) throw new Exception("Clipboard text lock failed");
        try { var b=new byte[size]; Marshal.Copy(p,b,0,size); return b; }
        finally { GlobalUnlock(h); }
    }
    void Lock() { if(!OpenClipboard(local)) throw new Exception("Clipboard busy; no retry/overwrite"); }
    void Guard() {
        if(sequence!=GetClipboardSequenceNumber() || owner!=GetClipboardOwner())
            throw new Exception("Clipboard changed concurrently; foreign update retained, no overwrite");
    }
    public void Copy(HistoryCells table,string expected,bool noCopy) {
        if(!CanRun || disposed) throw new Exception("Clipboard proof not armed");
        Lock(); try { Guard(); } finally { CloseClipboard(); }
        Exception failure=null;
        try { table.Message(0x301,0,0); } catch(Exception e) { failure=e; }
        uint after=GetClipboardSequenceNumber(); IntPtr afterOwner=GetClipboardOwner();
        bool modified=after!=sequence || afterOwner!=owner;
        if(modified) {
            if(afterOwner!=table.Table || ProxyUiSmoke.Owner(afterOwner)!=table.Pid)
                throw new Exception("Foreign clipboard update during copy; not overwritten/restored");
            // Record the owned write BEFORE opening so a busy post-copy lock
            // cannot lose the original snapshot's guarded restoration attempt.
            changed=true; sequence=after; owner=afterOwner;
        }
        Lock();
        try {
            Guard();
            if(failure!=null) throw failure;
            if(noCopy) {
                if(modified) throw new Exception("WM_COPY changed clipboard after active cell cleared");
                return;
            }
            if(!modified) throw new Exception("WM_COPY did not write the active cell");
            IntPtr data=GetClipboardData(13); ulong size=GlobalSize(data).ToUInt64();
            if(data==IntPtr.Zero || size<2 || size>Bound || size%2!=0) throw new Exception("Invalid Unicode clipboard result");
            byte[] bytes=Read(data,(int)size); int end=0;
            while(end+1<bytes.Length && (bytes[end]!=0 || bytes[end+1]!=0)) end+=2;
            if(end+1>=bytes.Length || new UnicodeEncoding(false,false,true).GetString(bytes,0,end)!=expected)
                throw new Exception("WM_COPY differs from exact active cell (no headers/other columns allowed)");
            Guard();
        } finally { CloseClipboard(); }
    }
    public void Dispose() {
        if(disposed) return; disposed=true;
        try {
            if(!changed) return;
            // Prepare ALL allocations before clearing. SetClipboardData transfers
            // only successful handles; every untransferred allocation is freed.
            var memory=new Dictionary<uint,IntPtr>();
            try {
                foreach(var entry in saved) {
                    IntPtr h=GlobalAlloc(2,new UIntPtr((uint)entry.Value.Length));
                    if(h==IntPtr.Zero) throw new Exception("Clipboard restore allocation failed");
                    memory.Add(entry.Key,h); IntPtr p=GlobalLock(h);
                    if(p==IntPtr.Zero) throw new Exception("Clipboard restore lock failed");
                    try { Marshal.Copy(entry.Value,0,p,entry.Value.Length); } finally { GlobalUnlock(h); }
                }
                Lock();
                try {
                    Guard();
                    if(!EmptyClipboard()) throw new Exception("Clipboard restore clear failed");
                    foreach(var format in new List<uint>(memory.Keys)) {
                        if(SetClipboardData(format,memory[format])==IntPtr.Zero) throw new Exception("Clipboard format restore failed");
                        memory[format]=IntPtr.Zero;
                    }
                    Status="PASS: original bounded text formats/empty clipboard restored";
                } finally { CloseClipboard(); }
            } finally { foreach(var h in memory.Values) if(h!=IntPtr.Zero) GlobalFree(h); }
        } finally { DestroyWindow(local); }
    }
}
'@
$joined = @($proxyDeclaration,$historyDeclaration,$cellDeclaration) -join "`n"
$usingPattern = '(?m)^using [^;\r\n]+;\r?$'
$usings = @([regex]::Matches($joined,$usingPattern) | ForEach-Object { $_.Value.Trim() } | Select-Object -Unique)
Add-Type -TypeDefinition (($usings -join "`n") + "`n" + [regex]::Replace($joined,$usingPattern,''))
$temp = Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-history-ui-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temp)
$child=$null; $mock=$null; $window=[IntPtr]::Zero
$overall=[Diagnostics.Stopwatch]::StartNew()
$script:case='startup'
$script:lastAction='none'
function Trace-Action([string]$Action) {
    $caller=(Get-PSCallStack)[2]
    $script:lastAction="$Action at $($caller.Location)"
    $script:case=$script:lastAction
    Write-Output ("STEP {0}: {1}" -f $overall.Elapsed.ToString('mm\:ss\.fff'),$script:lastAction)
}
function Wait([string]$Name,[scriptblock]$Test) {
    $script:case=$Name
    $timer=[Diagnostics.Stopwatch]::StartNew()
    while($timer.Elapsed.TotalSeconds -lt $StepTimeoutSeconds -and $overall.Elapsed.TotalSeconds -lt 300) {
        if($child.HasExited) { throw "Child exited: $Name" }
        if(& $Test) { return }
        Start-Sleep -Milliseconds 60
    }
    throw ("Timeout: $Name; lastAction=$script:lastAction; status=" + (Text 2325))
}
function Control([int]$Id) { [ProxyUiSmoke]::GetDlgItem($window,$Id) }
function Text([int]$Id) { [HistoryUi]::Text((Control $Id),[uint32]$child.Id) }
function Rows { [HistoryUi]::Value((Control 2337),[uint32]$child.Id,0x1004) }
function Operation {
    $values=@{}
    foreach($part in (Text 2354).Split(';')) {
        $pair=$part.Split('=')
        if($pair.Count -eq 2) { $values[$pair[0]]=[uint64]$pair[1] }
    }
    if($values.Count -ne 5) { throw 'Missing history operation state; build the current source before smoke' }
    return $values
}
function Ready {
    $state=Operation
    $state.idle -eq 1 -and [ProxyUiSmoke]::IsWindowEnabled((Control 2320)) -and [ProxyUiSmoke]::Dialog([uint32]$child.Id) -eq [IntPtr]::Zero
}
function Fence {
    # SendMessage can overtake posted choice notifications/BM_CLICK. A posted
    # owner-thread acknowledgement observes their effects, even for no-op choices.
    $before=(Operation).fence
    [ProxyUiSmoke]::Post($window,[uint32]$child.Id,0x111,2397)
    Wait 'history owner-thread queue fence' { (Operation).fence -gt $before }
}
function Wait-Idle {
    Fence
    Wait 'history truly idle' { Ready }
    if((Operation).rejected -ne 0) { throw ('History rejected an action; state=' + (Text 2354) + '; status=' + (Text 2325)) }
}
function Set-Field([int]$Id,[string]$Value) {
    Wait-Idle
    if(-not [ProxyUiSmoke]::IsWindowEnabled((Control $Id))) { throw "Disabled field $Id" }
    [ProxyUiSmoke]::SetText((Control $Id),[uint32]$child.Id,$Value)
}
function Click([int]$Id) {
    Trace-Action "Click $Id"
    Wait-Idle
    if(-not [ProxyUiSmoke]::IsWindowEnabled((Control $Id))) { throw "Disabled control $Id; case=$script:case; lastAction=$script:lastAction; status=$(Text 2325)" }
    [ProxyUiSmoke]::Post((Control $Id),[uint32]$child.Id,0xF5,0)
    Fence
}
function Combo([int]$Id,[uint32]$Index) {
    Trace-Action "Combo $Id index $Index"
    Wait-Idle
    $before=(Operation).sequence
    [ProxyUiSmoke]::Send((Control $Id),[uint32]$child.Id,0x14E,$Index)
    if([HistoryUi]::Value((Control $Id),[uint32]$child.Id,0x147) -ne $Index) { throw "CB_SETCURSEL/CB_GETCURSEL mismatch: $Id index $Index" }
    [ProxyUiSmoke]::Send($window,[uint32]$child.Id,0x111,($Id -bor (1 -shl 16)))
    Wait-Idle
    if($Id -in @(2333,2334,2335,2336) -and (Operation).sequence -ne ($before+1)) { throw "Choice $Id did not complete exactly one automatic query" }
}
function Choice-Key([int]$Id,[uint32]$Key,[int]$ExpectedIndex) {
    Trace-Action "Native choice key $Id key $Key"
    Wait-Idle
    $before=(Operation).sequence
    # Real native Tab keyboard input produces TCN_SELCHANGE, then choices::notify
    # posts CBN_SELCHANGE. Do not synthesize WM_COMMAND on this path.
    [ProxyUiSmoke]::Send((Control $Id),[uint32]$child.Id,0x100,$Key)
    Wait-Idle
    if([HistoryUi]::Value((Control $Id),[uint32]$child.Id,0x147) -ne $ExpectedIndex) { throw 'Native choice keyboard selection mismatch' }
    if((Operation).sequence -ne ($before+1)) { throw 'Posted native choice notification did not complete exactly one query' }
}
function Assert-Result([int]$Count) {
    Wait-Idle
    Wait "history result $Count" { (Ready) -and (Text 2325).Contains("匹配 $Count 请求") }
}
function Answer([bool]$Yes) {
    $script:dialog=[IntPtr]::Zero
    Wait 'confirmation' { $script:dialog=[ProxyUiSmoke]::Dialog([uint32]$child.Id); $dialog -ne [IntPtr]::Zero }
    $answered=$dialog
    $button=7; if($Yes) { $button=6 }
    [ProxyUiSmoke]::Post([ProxyUiSmoke]::GetDlgItem($dialog,$button),[uint32]$child.Id,0xF5,0)
    Wait 'confirmation closed' { [ProxyUiSmoke]::Dialog([uint32]$child.Id) -ne $answered }
    Wait-Idle
}
function Answer-No { Answer $false }
function Query([int]$Count) {
    Click 2320
    Assert-Result $Count
}
function Save-Export([string]$Path,[int]$Count,[bool]$All) {
    $choice=0; if($All) { $choice=1 }
    Combo 2353 $choice
    Click 2323
    $script:dialog=[IntPtr]::Zero
    Wait 'export privacy confirmation' { $script:dialog=[ProxyUiSmoke]::Dialog([uint32]$child.Id); $dialog -ne [IntPtr]::Zero }
    $confirmation=$dialog
    [ProxyUiSmoke]::Post([ProxyUiSmoke]::GetDlgItem($dialog,6),[uint32]$child.Id,0xF5,0)
    Wait 'native CSV save dialog' {
        $script:dialog=[ProxyUiSmoke]::Dialog([uint32]$child.Id)
        $dialog -ne [IntPtr]::Zero -and $dialog -ne $confirmation -and @([ProxyUiSmoke]::Children($dialog,'Edit') | Where-Object { [ProxyUiSmoke]::IsWindowVisible($_) }).Count -gt 0
    }
    $pathEdit=@([ProxyUiSmoke]::Children($dialog,'Edit') | Where-Object { [ProxyUiSmoke]::IsWindowVisible($_) })[-1]
    [ProxyUiSmoke]::SetText($pathEdit,[uint32]$child.Id,$Path)
    [ProxyUiSmoke]::Post([ProxyUiSmoke]::GetDlgItem($dialog,1),[uint32]$child.Id,0xF5,0)
    Wait 'worker CSV result' { (Test-Path -LiteralPath $Path) -and (Text 2325).Contains('CSV 已导出') -and (Ready) }
    if(@(Import-Csv -LiteralPath $Path).Count -ne $Count) { throw 'CSV range/row count mismatch' }
    $data=[IO.File]::ReadAllText($Path)
    if($data.Contains($a) -or $data.Contains($b) -or $data.Contains($id)) { throw 'CSV raw credential or HMAC leak' }
    if((Text 2325).Contains('仅当前页 true') -eq $All) { throw 'CSV result range label mismatch' }
}
function Test-CellTable([string]$View) {
    Trace-Action "Cell table ${View}: local clicks, keys and cancellable native menus"
    Wait-Idle
    $table=[HistoryCells]::new($child.Handle,[uint32]$child.Id,(Control 2337))
    try {
        if((Rows) -lt 2) { throw "Cell $View fixture needs at least two native rows" }
        # Read real cells through bounded remote LVITEMW, not the legacy text view.
        $first=$table.Cell(0,1); $second=$table.Cell(1,1)
        Write-Output "CELL $View remote UTF-16 reads: row0=$($first.Length) units; row1=$($second.Length) units (contents not logged)"
        $table.ClickCell(0,1); Fence
        if($table.Selected() -ne 0) { throw "Cell $View subitem click did not select row 0" }
        Write-Output ($table.Menu($false,0,1,$true))
        Fence
        foreach($key in @(0x27,0x25)) {
            [void]$table.Message(0x100,$key,1)
            [void]$table.Message(0x101,$key,3221225473)
            if($table.Selected() -ne 0) { throw "Cell $View left/right changed native row" }
            Write-Output ($table.Menu($true,0,0,$true))
            Fence
        }
        [void]$table.Message(0x100,0x28,1)
        [void]$table.Message(0x101,0x28,3221225473)
        if($table.Selected() -ne 1) { throw "Cell $View native Down did not move row" }
        Write-Output ($table.Menu($true,0,0,$true))
        Fence
        if($View -eq 'groups') {
            $table.ClickBlank(); Fence
            Write-Output ($table.Menu($true,0,0,$false))
            Fence
            $table.ClickCell(0,1); Fence # Refresh must start with a live target.
        }
        Query 53
        $table.SelectRow(0) # Native row only: deliberately do not choose a column.
        Write-Output ($table.Menu($true,0,0,$false))
        Fence
        Write-Output "PASS: $View subitem selects row; left/right preserve row; Down moves row and preserves a copy target; refresh plus native row-only selection leaves copy disabled"
        Write-Output "LIMIT: $View exact active-column contents require optional clipboard proof; menu command is only observed/cancelled, never invoked"
        if($VerifyCellClipboard) { Test-CellClipboard $table $View }
        else { Write-Output "NOT RUN: $View WM_COPY/content/no-write proof (enable -VerifyCellClipboard); default does not open or write clipboard" }
    } finally { $table.Dispose() }
}
function Test-CellClipboard([HistoryCells]$Table,[string]$View) {
    Trace-Action "Opt-in $View protected cell WM_COPY"
    $proof=[HistoryCellClipboard]::new()
    try {
        if(-not $proof.CanRun) { Write-Output "$View $($proof.Status)"; return }
        # Change ONLY our temporary child's in-memory presentation while hidden.
        # No DB/alias/user fixture is edited; a real query reconstructs all cells.
        $unicode='cell-'+[char]::ConvertFromUtf32(0x1F600)+('中文'+[char]::ConvertFromUtf32(0x1F980))*2048+'-end'
        $other='other-column-'+[Guid]::NewGuid().ToString('N')
        try {
            $Table.Visible($false)
            $Table.SetCell(0,1,$unicode); $Table.SetCell(0,2,$other)
            if($Table.Cell(0,1) -cne $unicode) { throw 'Remote Unicode fixture was truncated before copying' }
        } finally { $Table.Visible($true) }
        $Table.ClickCell(0,1); Fence
        $proof.Copy($Table,$unicode,$false)
        [void]$Table.Message(0x100,0x27,1)
        $proof.Copy($Table,$other,$false)
        [void]$Table.Message(0x100,0x25,1)
        $proof.Copy($Table,$unicode,$false)
        [void]$Table.Message(0x100,0x28,1)
        $proof.Copy($Table,$Table.Cell(1,1),$false)
        # Empty strings are real copy targets, not 'no selection'.
        try { $Table.Visible($false); $Table.SetCell(0,1,'') }
        finally { $Table.Visible($true) }
        $Table.ClickCell(0,1); Fence
        $proof.Copy($Table,'',$false)
        try {
            $Table.Visible($false)
            $proof.Copy($Table,'',$true)
        } finally { $Table.Visible($true) }
        if($View -eq 'groups') {
            $Table.ClickBlank(); Fence
            $proof.Copy($Table,'',$true)
            $Table.ClickCell(0,1); Fence
        }
        Query 53
        $Table.SelectRow(0)
        $proof.Copy($Table,'',$true)
        Write-Output "PASS: $View WM_COPY exact long Unicode/emoji, left/right columns, native Down retaining column, empty cell, hidden no-write and refresh no-stale-target"
    } finally {
        # Restore while the owning child/clipboard HWND is still alive. The guard
        # refuses to overwrite any concurrent sequence/owner update.
        try { $proof.Dispose(); Write-Output "$View $($proof.Status)" }
        finally { Query 53 }
    }
}
function StartGui {
    $script:child=Start-Process -FilePath $exe -ArgumentList @('--config-dir',('"'+$temp+'"')) -PassThru
    Wait 'window loaded' { $script:window=[ProxyUiSmoke]::Find([uint32]$child.Id); $window -ne [IntPtr]::Zero -and [ProxyUiSmoke]::IsWindowEnabled((Control 2003)) }
    [ProxyUiSmoke]::SelectTab($window,[uint32]$child.Id,1)
    Wait 'history scope received from controller' { (Text 2330).Contains('数据目录已变化') }
}
function StopGui {
    [ProxyUiSmoke]::Post($window,[uint32]$child.Id,0x111,1002)
    if(-not $child.WaitForExit(38000)) { throw 'Graceful exit timeout' }
    if($child.ExitCode -ne 0) { throw 'Nonzero child exit' }
    $child.Dispose();$script:child=$null
}
try {
    $mock=[HistoryMock]::new()
    $port=[ProxyUiSmoke]::FreePort()
    while($port -eq $mock.Port) { $port=[ProxyUiSmoke]::FreePort() }
    [IO.File]::WriteAllText((Join-Path $temp 'config.toml'), "schema_version = 1`n[upstream]`nurl = 'http://127.0.0.1:$($mock.Port)/v1'`n[listen]`naddress = '127.0.0.1'`nport = $port`n[proxy]`nauto_start = true`n")
    StartGui
    $a='synthetic-client-a-'+[Guid]::NewGuid().ToString('N')
    $b='synthetic-client-b-'+[Guid]::NewGuid().ToString('N')
    for($i=0;$i -lt 52;$i++) { [HistoryRequest]::Get($port,$a) }
    [HistoryRequest]::Post($port,$b)
    Query 53
    if(-not (Text 2330).Contains('总 795')) { throw 'True token summary mismatch' }
    if(-not [ProxyUiSmoke]::IsWindowVisible((Control 2332))) { throw 'Native chart not visible' }
    if([ProxyUiSmoke]::Class((Control 2337)) -ne 'SysListView32' -or (Rows) -ne 50 -or -not [ProxyUiSmoke]::IsWindowVisible((Control 2337))) { throw 'Native details table absent/empty' }
    if([ProxyUiSmoke]::IsWindowVisible((Control 2331))) { throw 'Legacy text overlaps native table' }
    if(-not (Text 2331).Contains('local-requested-model')) { throw 'Actual request model not collected' }
    Test-CellTable 'details'
    Click 2318 # Expanded filters replace the chart, not the native table.
    Wait 'advanced filters without table overlap' { [ProxyUiSmoke]::IsWindowVisible((Control 2307)) -and -not [ProxyUiSmoke]::IsWindowVisible((Control 2332)) -and [ProxyUiSmoke]::IsWindowVisible((Control 2337)) }
    $detail=Text 2331
    $ids=[regex]::Matches($detail,'[0-9a-f]{64}')
    if($ids.Count -eq 0) { throw 'No opaque credential identity in real details' }
    $id=$ids[0].Value
    Set-Field 2307 $id
    # Newest record is b: query ordering determines which actual identity is copied.
    Click 2320
    Wait 'credential-filtered query' { [ProxyUiSmoke]::IsWindowEnabled((Control 2320)) -and (Text 2325).Contains('历史已查询') }
    if(-not ((Text 2325).Contains('匹配 1 请求') -or (Text 2325).Contains('匹配 52 请求'))) { throw ('Credential filter ignored: ' + (Text 2325) + '; ID=' + $id + '; draft=' + (Text 2307)) }
    Set-Field 2307 ''
    $upstreamId=[regex]::Match($detail,'\| ([0-9a-f]{64})\r?\n').Groups[1].Value
    if(-not $upstreamId) { throw 'Missing actual upstream credential identity' }
    Set-Field 2308 $upstreamId
    Click 2320
    Wait 'upstream-credential-filtered query' { [ProxyUiSmoke]::IsWindowEnabled((Control 2320)) -and (Text 2325).Contains('历史已查询') }
    if(-not ((Text 2325).Contains('匹配 1 请求') -or (Text 2325).Contains('匹配 52 请求'))) { throw 'Upstream credential filter ignored' }
    Set-Field 2308 ''
    Set-Field 2304 '127.0.0.1'; Set-Field 2305 'local-usage-model'
    Set-Field 2309 "http://127.0.0.1:$($mock.Port)"
    Combo 2310 0; Combo 2312 1; Set-Field 2313 '200'
    Combo 2314 1; Combo 2315 3; Combo 2316 5
    Query 53
    if(-not (Text 2330).Contains('分组')) { throw 'No real grouped summary' }
    Click 2322
    Wait 'second details page' { (Text 2325).Contains('offset 50 / 3 行') }
    Click 2321
    Wait 'first details page' { (Text 2325).Contains('offset 0 / 50 行') }
    Set-Field 2305 'no-such-local-model'; Query 0
    if(-not (Text 2330).Contains('总 未知')) { throw 'Missing token usage rendered as zero' }
    Set-Field 2305 'local-usage-model'; Query 53
    Combo 2301 1; Query 53
    Combo 2301 2; Query 53
    Combo 2301 3; Query 53
    Combo 2301 4
    Set-Field 2302 ([DateTime]::Now.ToString('yyyy-MM-dd'))
    Set-Field 2303 ([DateTime]::Now.AddDays(1).ToString('yyyy-MM-dd'))
    Query 53
    # Request-model + new dimensions, timezone/granularity and backend sorting.
    Set-Field 2306 'local-requested-model'; Query 1
    Set-Field 2306 ''; Query 53
    Combo 2301 0
    Combo 2333 0
    Choice-Key 2333 0x27 1
    Assert-Result 53
    if(-not (Text 2330).Contains('时区 Local')) { throw 'Native timezone notification did not apply Local query' }
    Choice-Key 2333 0x25 0
    Assert-Result 53
    Choice-Key 2334 0x27 1
    Assert-Result 53
    if(-not (Text 2330).Contains('趋势 Hour')) { throw 'Native granularity notification did not apply Hour query' }
    Choice-Key 2334 0x25 0
    Assert-Result 53
    foreach($zone in @(0,1)) {
        Combo 2333 $zone
        Combo 2301 1; Query 53
        Combo 2301 4
        $now=[DateTime]::UtcNow; if($zone -eq 1) { $now=[DateTime]::Now }
        Set-Field 2302 ($now.ToString('yyyy-MM-dd'))
        Set-Field 2303 ($now.AddDays(1).ToString('yyyy-MM-dd'))
        Query 53
        Combo 2301 0
        foreach($grain in @(0,1,2)) {
            Combo 2334 $grain; Assert-Result 53
            if(-not (Text 2330).Contains('趋势')) { throw 'No backend trend labels' }
        }
    }
    Combo 2333 0; Combo 2301 0
    Combo 2314 9; Combo 2315 10; Combo 2316 11; Query 53
    Combo 2338 1
    if((Rows) -ne 2 -or -not [ProxyUiSmoke]::IsWindowVisible((Control 2337))) { throw 'Grouped native table mismatch' }
    Test-CellTable 'groups'
    for($sort=0;$sort -lt 8;$sort++) {
        Combo 2336 $sort
        Wait 'group sort backend result' { (Ready) -and (Text 2325).Contains('匹配 53 请求') }
        if((Rows) -ne 2) { throw 'Sorting changed real aggregate groups' }
    }
    Combo 2338 0
    for($sort=0;$sort -lt 4;$sort++) {
        Combo 2335 $sort
        Wait 'details sort backend result' { (Ready) -and (Text 2325).Contains('匹配 53 请求') }
        if((Rows) -ne 50) { throw 'Details sorting lost rows' }
    }
    Combo 2338 2
    if(-not [ProxyUiSmoke]::IsWindowVisible((Control 2330)) -or [ProxyUiSmoke]::IsWindowVisible((Control 2337))) { throw 'Accessible summary view overlaps table' }
    Combo 2338 3
    if(-not [ProxyUiSmoke]::IsWindowVisible((Control 2331))) { throw 'Accessible metadata text unavailable' }
    Combo 2338 0
    Save-Export (Join-Path $temp 'current-page.csv') 50 $false
    Save-Export (Join-Path $temp 'all-matches.csv') 53 $true
    $allRows=@(Import-Csv -LiteralPath (Join-Path $temp 'all-matches.csv'))
    if(@($allRows | Where-Object { $_.requested_model -eq 'local-requested-model' }).Count -ne 1) { throw 'All export omitted real request model' }
    if($allRows[0].PSObject.Properties.Name -contains 'client' -or $allRows[0].PSObject.Properties.Name -contains 'ip') { throw 'All export exposes private identity columns' }
    # Cancelling confirmation must never enter the file chooser or write a file.
    Click 2323
    Answer-No
    Click 2324
    Wait 'real recording counters' { (Text 2325).Contains('历史记录状态') -and [ProxyUiSmoke]::IsWindowEnabled((Control 2320)) }
    Click 2317
    Wait 'draft reset completed' { (Text 2325).Contains('草稿已重置') }
    if((Text 2304) -ne '' -or (Text 2309) -ne '' -or -not (Text 2325).Contains('草稿已重置')) { throw 'Reset did not clear draft-only filters' }
    if(-not (Text 2330).Contains('总 795')) { throw 'Reset discarded applied history report' }
    Query 53
    Click 2339
    Wait 'storage stats' { (Ready) -and (Text 2351).Contains('53 条记录') }
    if([ProxyUiSmoke]::IsWindowVisible((Control 2337))) { throw 'Management overlaps table' }
    Set-Field 2341 $id; Set-Field 2342 'smoke-client-label'; Click 2343
    Wait 'alias save and report refresh' { (Ready) -and (Text 2331).Contains('smoke-client-label') }
    if(-not (Text 2330).Contains('总 795')) { throw 'Alias changed real usage' }
    Click 2344
    Wait 'alias removal and refresh' { (Ready) -and -not (Text 2331).Contains('smoke-client-label') }
    Set-Field 2342 'smoke-client-label'; Click 2343
    Wait 'alias restored for persistence' { (Ready) -and (Text 2331).Contains('smoke-client-label') }
    Set-Field 2342 'sk-synthetic-not-a-real-key'; Click 2343
    Wait 'secret-like alias rejected' { (Text 2351).Contains('请勿粘贴密钥') }
    Set-Field 2342 'smoke-client-label'
    Set-Field 2346 '90'; Set-Field 2347 '60'
    Click 2348; Answer $false
    Click 2345
    Wait 'policy cancel preserved real policy' { (Ready) -and (Text 2347) -eq '100000' }
    Set-Field 2346 '90'; Set-Field 2347 '60'
    Click 2348; Answer $true
    Wait 'policy saved and applied' { (Ready) -and (Text 2325).Contains('历史策略已保存并应用') }
    if(-not [IO.File]::ReadAllText((Join-Path $temp 'config.toml')).Contains('max_records = 60')) { throw 'Policy was not persisted' }
    Click 2350
    Wait 'backend clear preview' { (Ready) -and (Text 2351).Contains('53 条') -and [ProxyUiSmoke]::IsWindowEnabled((Control 2352)) }
    Click 2352; Answer $false
    Click 2345
    Wait 'clear cancellation kept records' { (Ready) -and (Text 2351).Contains('53 条记录') }
    Click 2339
    Query 53
    Combo 2300 1
    Wait 'session subview visible' { [ProxyUiSmoke]::IsWindowVisible((Control 2020)) -and -not [ProxyUiSmoke]::IsWindowVisible((Control 2330)) }
    Combo 2300 0
    if(-not (Text 2330).Contains('总 795')) { throw 'Session refresh overwrote history' }
    StopGui
    StartGui
    Query 53
    if(-not (Text 2330).Contains('总 795')) { throw 'Persisted totals lost after restart' }
    if(-not (Text 2331).Contains('smoke-client-label')) { throw 'Alias persistence lost after restart' }
    Click 2339
    Wait 'persisted policy storage' { (Ready) -and (Text 2347) -eq '60' }
    Click 2350
    Wait 'clear preview after restart' { (Ready) -and [ProxyUiSmoke]::IsWindowEnabled((Control 2352)) }
    Click 2352; Answer $true
    Wait 'clear completed' { (Ready) -and (Text 2325).Contains('历史已清空') }
    Click 2345
    Wait 'clear keeps aliases by default' { (Ready) -and (Text 2351).Contains('0 条记录') -and (Text 2351).Contains('1 个别名') }
    [HistoryRequest]::Post($port,$b)
    Click 2339; Query 1
    Click 2339
    Wait 'post-clear recording continued' { (Ready) -and (Text 2351).Contains('1 条记录') }
    Click 2350
    Wait 'preview before checkbox change' { (Ready) -and [ProxyUiSmoke]::IsWindowEnabled((Control 2352)) }
    Click 2349
    Wait 'changed alias scope invalidates preview' { -not [ProxyUiSmoke]::IsWindowEnabled((Control 2352)) }
    Click 2350
    Wait 'explicit alias removal preview' { (Ready) -and (Text 2351).Contains('true') -and [ProxyUiSmoke]::IsWindowEnabled((Control 2352)) }
    Click 2352; Answer $true
    Wait 'records and aliases cleared' { (Ready) -and (Text 2325).Contains('历史已清空') }
    Click 2345
    Wait 'zero aliases verified by worker' { (Ready) -and (Text 2351).Contains('0 条记录') -and (Text 2351).Contains('0 个别名') }
    StopGui
    Write-Output 'PASS: queue-fenced idle with zero rejected commands, native asynchronous choice keyboard notifications, native details/group ListView, 53 local requests including POST model, credential/combined filters, UTC/local ranges, hour/day/month labels, backend sorting, paging, page/all worker CSV, alias CRUD/restart, policy confirmation/persistence, preview/cancel/clear, alias opt-in removal and continued recording.'
    Write-Output 'CELL LIMIT: any NOT RUN above is skipped acceptance, not a complete cell-copy pass; no actual menu-command execution or Ctrl+C proof.'
    Write-Output 'NOT RUN: Ctrl+C clipboard proof; HWND-local PostMessage Ctrl/C alone cannot reliably set target GetKeyState. No global SendInput/keybd_event or remote code/thread injection used. Menu execution is not tested; appearance/caption/enablement/cancellation only.'
    Write-Output 'LIMITS: synthetic local metadata only; DST gap/fold checked by Rust injectable-zone unit tests, not by changing OS timezone. No real Startup changes, physical input, multi-monitor or screen-reader acceptance.'
} catch {
    Write-Output "FAIL: case=$script:case; lastAction=$script:lastAction"
    Write-Output $_.InvocationInfo.PositionMessage
    Write-Output $_.ScriptStackTrace
    if($window -ne [IntPtr]::Zero -and $null -ne $child -and -not $child.HasExited) {
        Write-Output ("HISTORY STATUS: " + (Text 2325))
        Write-Output ("OPERATION: " + (Text 2354))
        Write-Output ("QUERY ENABLED: " + [ProxyUiSmoke]::IsWindowEnabled((Control 2320)))
    }
    throw
} finally {
    if($null -ne $child) { if(-not $child.HasExited) { $child.Kill(); [void]$child.WaitForExit(10000) }; $child.Dispose() }
    if($null -ne $mock) { $mock.Dispose() }
    if(Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
}
