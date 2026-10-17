# Windows PowerShell 5.1, interactive Windows 10/11 desktop; no Cargo or network.
# Only our pinned child is automated. Existing user instances are never sent IPC.
[CmdletBinding()]
param([string]$ExePath)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $PSScriptRoot '..\target\status-bar-validation\target\debug\ai-relay.exe'
}

# Import C# declarations ONLY, never dot-source or execute either older test.
$proxySource = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'smoke-proxy-ui.ps1'))
$proxyDeclaration = [regex]::Match($proxySource, "(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@")
$claritySource = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'smoke-ui-clarity.ps1'))
$clarityDeclaration = [regex]::Match($claritySource, "(?s)Add-Type -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll -TypeDefinition @'\r?\n(.*?)\r?\n'@")
if (-not $proxyDeclaration.Success -or -not $clarityDeclaration.Success) { throw 'Missing existing native helper declarations' }
# Compile together: Windows PS cannot reference another Add-Type in-memory DLL.
if (('ProxyUiSmoke' -as [type]) -or ('ClarityNative' -as [type]) -or ('StatusBarNative' -as [type])) {
    throw 'Run in a fresh powershell.exe -NoProfile process (native helper names already loaded)'
}
$statusDeclaration = @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using Accessibility;
public static class StatusBarNative {
    [StructLayout(LayoutKind.Sequential)] public struct Rect {
        public int Left, Top, Right, Bottom;
        public int Width { get { return Right-Left; } }
        public int Height { get { return Bottom-Top; } }
        public override string ToString() { return String.Format("{0},{1} {2}x{3}",Left,Top,Width,Height); }
    }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
    [StructLayout(LayoutKind.Sequential)] struct MonitorInfo { public uint Size; public Rect Monitor, Work; public uint Flags; }
    delegate bool EnumProc(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr window,out Rect rect);
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr window,out Rect rect);
    [DllImport("user32.dll")] static extern bool ScreenToClient(IntPtr window,ref Point point);
    [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr window,ref Point point);
    [DllImport("user32.dll")] public static extern IntPtr GetParent(IntPtr window);
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr window,uint relation);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr window,uint flags);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback,IntPtr parameter);
    [DllImport("user32.dll")] static extern IntPtr GetThreadDpiAwarenessContext();
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern int GetAwarenessFromDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr window,IntPtr after,int x,int y,int width,int height,uint flags);
    [DllImport("user32.dll")] static extern IntPtr MonitorFromWindow(IntPtr window,uint flags);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern bool GetMonitorInfoW(IntPtr monitor,ref MonitorInfo info);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x,int y);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] static extern uint GetGuiResources(IntPtr process,uint flags);
    [DllImport("kernel32.dll")] static extern uint GetProcessId(IntPtr process);
    [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr handle,uint milliseconds);
    [DllImport("oleacc.dll")] static extern int AccessibleObjectFromWindow(IntPtr window,uint objectId,ref Guid iid,[MarshalAs(UnmanagedType.Interface)] out IAccessible accessible);
    public static IntPtr EnterDpi() {
        IntPtr previous=SetThreadDpiAwarenessContext(new IntPtr(-4));
        if(previous==IntPtr.Zero || GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext())!=2)
            throw new Exception("Cannot establish physical per-monitor-v2 coordinates");
        return previous;
    }
    public static void RestoreDpi(IntPtr previous) { SetThreadDpiAwarenessContext(previous); }
    public static void CheckProcess(IntPtr handle,uint pid) {
        if(handle==IntPtr.Zero || GetProcessId(handle)!=pid || WaitForSingleObject(handle,0)!=258)
            throw new Exception("Pinned process handle is no longer the live child");
    }
    public static uint Gdi(IntPtr handle,uint pid) {
        CheckProcess(handle,pid); uint count=GetGuiResources(handle,0);
        if(count==0) throw new Exception("Cannot query child GDI resources");
        return count;
    }
    public static Rect Client(IntPtr window,uint pid) {
        ProxyUiSmoke.CheckOwner(window,pid); var previous=EnterDpi();
        try { Rect r; if(!GetClientRect(window,out r)) throw new Exception("GetClientRect failed"); return r; }
        finally { RestoreDpi(previous); }
    }
    public static Rect InParent(IntPtr window,IntPtr parent,uint pid) {
        ProxyUiSmoke.CheckOwner(window,pid); ProxyUiSmoke.CheckOwner(parent,pid); var previous=EnterDpi();
        try {
            Rect r; if(!GetWindowRect(window,out r)) throw new Exception("GetWindowRect failed");
            var a=new Point { X=r.Left,Y=r.Top }; var b=new Point { X=r.Right,Y=r.Bottom };
            if(!ScreenToClient(parent,ref a) || !ScreenToClient(parent,ref b)) throw new Exception("ScreenToClient failed");
            return new Rect { Left=a.X,Top=a.Y,Right=b.X,Bottom=b.Y };
        } finally { RestoreDpi(previous); }
    }
    public static IntPtr[] VisibleSiblings(IntPtr parent,IntPtr status,uint pid) {
        ProxyUiSmoke.CheckOwner(parent,pid); ProxyUiSmoke.CheckOwner(status,pid);
        var found=new List<IntPtr>(); int visited=0;
        for(IntPtr h=GetWindow(parent,5); h!=IntPtr.Zero; h=GetWindow(h,2)) {
            if(++visited>4096) throw new Exception("Sibling enumeration limit exceeded");
            ProxyUiSmoke.CheckOwner(h,pid);
            if(h!=status && ProxyUiSmoke.IsWindowVisible(h)) found.Add(h);
        }
        return found.ToArray();
    }
    public static IntPtr[] Tooltips(IntPtr status,uint pid) {
        ProxyUiSmoke.CheckOwner(status,pid); var found=new List<IntPtr>();
        EnumWindows(delegate(IntPtr h,IntPtr unused) {
            // A popup created with a child owner may be owned by its top-level
            // ancestor instead. Actual association is proven by hovered text.
            IntPtr owner=GetWindow(h,4);
            if(ProxyUiSmoke.Owner(h)==pid && ProxyUiSmoke.Class(h)=="tooltips_class32" &&
                (owner==status || owner==GetAncestor(status,2))) found.Add(h);
            return true;
        },IntPtr.Zero);
        return found.ToArray();
    }
    [StructLayout(LayoutKind.Sequential)] struct ToolInfo {
        public uint Size,Flags; public IntPtr Window,Id; public Rect Bounds;
        public IntPtr Instance,Text,Parameter,Reserved;
    }
    [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr OpenProcess(uint access,bool inherit,uint pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr VirtualAllocEx(IntPtr process,IntPtr address,UIntPtr size,uint allocation,uint protection);
    [DllImport("kernel32.dll")] static extern bool VirtualFreeEx(IntPtr process,IntPtr address,UIntPtr size,uint kind);
    [DllImport("kernel32.dll")] static extern bool WriteProcessMemory(IntPtr process,IntPtr address,byte[] bytes,UIntPtr count,out UIntPtr written);
    [DllImport("kernel32.dll")] static extern bool ReadProcessMemory(IntPtr process,IntPtr address,byte[] bytes,UIntPtr count,out UIntPtr read);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr window,uint message,UIntPtr w,IntPtr l,uint flags,uint timeout,out UIntPtr result);
    public static string VerifyTooltipPointer(IntPtr pinned,uint pid,IntPtr tooltip,IntPtr status,string expected) {
        CheckProcess(pinned,pid); ProxyUiSmoke.CheckOwner(tooltip,pid); ProxyUiSmoke.CheckOwner(status,pid);
        if(IntPtr.Size!=8) throw new Exception("Use 64-bit Windows PowerShell for the x64 target");
        // VM operation/read/write + query-limited + SYNCHRONIZE for CheckProcess.
        IntPtr process=OpenProcess(0x101038,false,pid), remote=IntPtr.Zero; bool completed=true;
        if(process==IntPtr.Zero) throw new Exception("Cannot open owned child VM for tooltip inspection");
        try {
            CheckProcess(pinned,pid); CheckProcess(process,pid);
            int textBytes=checked((expected.Length+1)*2);
            int size=Marshal.SizeOf(typeof(ToolInfo));
            remote=VirtualAllocEx(process,IntPtr.Zero,new UIntPtr((uint)(size+textBytes)),0x3000,4);
            if(remote==IntPtr.Zero) throw new Exception("Remote tooltip allocation failed");
            IntPtr textAddress=IntPtr.Add(remote,size);
            var tool=new ToolInfo { Size=(uint)size,Window=status,Id=status,Text=textAddress };
            var bytes=new byte[size]; IntPtr local=Marshal.AllocHGlobal(size);
            try { Marshal.StructureToPtr(tool,local,false); Marshal.Copy(local,bytes,0,size); }
            finally { Marshal.FreeHGlobal(local); }
            UIntPtr count,result;
            if(remote==IntPtr.Zero || !WriteProcessMemory(process,remote,bytes,new UIntPtr((uint)size),out count) || count.ToUInt64()!=(ulong)size)
                throw new Exception("Remote TOOLINFO allocation/write failed");
            // Both TOOLINFO and its output string are TARGET allocations, not
            // local pointers. Win10+ wParam bounds TTM_GETTEXTW's output buffer.
            completed=false;
            if(SendMessageTimeoutW(tooltip,0x438,new UIntPtr((uint)(expected.Length+1)),remote,0x22,2000,out result)==IntPtr.Zero)
                throw new Exception("Remote TTM_GETTEXTW timed out; allocation retained until child exit");
            completed=true;
            bytes=new byte[textBytes];
            if(!ReadProcessMemory(process,textAddress,bytes,new UIntPtr((uint)bytes.Length),out count) || count.ToUInt64()!=(ulong)bytes.Length)
                throw new Exception("Cannot read complete native tooltip text");
            string actual=System.Text.Encoding.Unicode.GetString(bytes);
            if(actual!=expected+"\0") throw new Exception("TTM_GETTEXTW text differs from exact original");
            if(SendMessageTimeoutW(tooltip,0x435,UIntPtr.Zero,remote,0x22,2000,out result)==IntPtr.Zero)
                throw new Exception("TTM_GETTOOLINFOW diagnostic timed out");
            bytes=new byte[size];
            if(!ReadProcessMemory(process,remote,bytes,new UIntPtr((uint)size),out count)) throw new Exception("Tool diagnostic read failed");
            local=Marshal.AllocHGlobal(size);
            try { Marshal.Copy(bytes,0,local,size); tool=(ToolInfo)Marshal.PtrToStructure(local,typeof(ToolInfo)); }
            finally { Marshal.FreeHGlobal(local); }
            return String.Format("TTM_GETTEXTW exact {0} UTF-16 units; target-only buffers; tool flags=0x{1:X}; hwnd match={2}; id match={3}",expected.Length,tool.Flags,tool.Window==status,tool.Id==status);
        } finally {
            if(remote!=IntPtr.Zero && completed) VirtualFreeEx(process,remote,UIntPtr.Zero,0x8000);
            CloseHandle(process);
        }
    }
    public static string TooltipName(IntPtr tooltip,uint pid) {
        ProxyUiSmoke.CheckOwner(tooltip,pid);
        // MSAA proxies marshal inside oleacc: no client TOOLINFO/text pointer is
        // passed to a foreign HWND. Its live name reads the tooltip's retained
        // lpszText, testing the actual text pointer rather than just tool count.
        var task=Task.Run(delegate {
            ProxyUiSmoke.CheckOwner(tooltip,pid);
            Guid iid=new Guid("618736E0-3C3D-11CF-810C-00AA00389B71"); IAccessible accessible;
            int hr=AccessibleObjectFromWindow(tooltip,0xFFFFFFFC,ref iid,out accessible);
            if(hr<0) Marshal.ThrowExceptionForHR(hr);
            if(accessible==null) throw new Exception("Tooltip MSAA object unavailable");
            try { return accessible.get_accName(0); }
            finally { Marshal.ReleaseComObject(accessible); }
        });
        if(!task.Wait(3000)) throw new Exception("Tooltip accessibility exceeded three seconds");
        return task.GetAwaiter().GetResult();
    }
    public static void Hover(IntPtr window,IntPtr status,uint pid) {
        ProxyUiSmoke.CheckOwner(window,pid); ProxyUiSmoke.CheckOwner(status,pid); var previous=EnterDpi();
        try {
            Rect r=Client(status,pid); var p=new Point { X=r.Width/2,Y=r.Height/2 };
            if(!ClientToScreen(status,ref p)) throw new Exception("ClientToScreen failed");
            SetForegroundWindow(window);
            var away=new Point { X=8,Y=8 };
            if(!ClientToScreen(window,ref away) || !SetCursorPos(away.X,away.Y)) throw new Exception("Cannot reset native hover");
            System.Threading.Thread.Sleep(100);
            if(!SetCursorPos(p.X,p.Y) || WindowFromPoint(p)!=status)
                throw new Exception("Status hover obstructed by another window; refusing foreign input");
            // Native TTF_SUBCLASS consumes mouse movement and starts its own timer.
            ClarityNative.Message(status,pid,0x200,0,((r.Height/2)<<16)|((r.Width/2)&65535));
        } finally { RestoreDpi(previous); }
    }
    public static void RestoreCursor(Point point) { SetCursorPos(point.X,point.Y); }
    public static Rect Resize(IntPtr window,uint pid,bool smaller) {
        ProxyUiSmoke.CheckOwner(window,pid); var previous=EnterDpi();
        try {
            var info=new MonitorInfo { Size=(uint)Marshal.SizeOf(typeof(MonitorInfo)) };
            if(!GetMonitorInfoW(MonitorFromWindow(window,2),ref info)) throw new Exception("Monitor work area unavailable");
            uint dpi=ClarityNative.GetDpiForWindow(window);
            if(dpi==0) throw new Exception("Window DPI unavailable");
            int width=Math.Min(info.Work.Width,(int)Math.Round((smaller?780:940)*dpi/96.0));
            int height=Math.Min(info.Work.Height,(int)Math.Round((smaller?890:1020)*dpi/96.0));
            // Async positioning avoids an unbounded cross-thread synchronous wait.
            if(!SetWindowPos(window,IntPtr.Zero,info.Work.Left,info.Work.Top,width,height,0x4014))
                throw new Exception("Async SetWindowPos failed");
            return new Rect { Left=info.Work.Left,Top=info.Work.Top,Right=info.Work.Left+width,Bottom=info.Work.Top+height };
        } finally { RestoreDpi(previous); }
    }
    public static Rect Outer(IntPtr window,uint pid) {
        ProxyUiSmoke.CheckOwner(window,pid); var previous=EnterDpi();
        try { Rect r; if(!GetWindowRect(window,out r)) throw new Exception("GetWindowRect failed"); return r; }
        finally { RestoreDpi(previous); }
    }
}
'@
$declarations = $proxyDeclaration.Groups[1].Value + "`n" + $clarityDeclaration.Groups[1].Value + "`n" + $statusDeclaration
$usingPattern = '(?m)^using [^;\r\n]+;\r?$'
$usings = @([regex]::Matches($declarations,$usingPattern) | ForEach-Object { $_.Value.Trim() } | Select-Object -Unique)
$combined = ($usings -join "`n") + "`n" + [regex]::Replace($declarations,$usingPattern,'')
Add-Type -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll,Accessibility.dll -TypeDefinition $combined
$session = [Diagnostics.Process]::GetCurrentProcess().SessionId
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$mutexName = "Local\AIRelay.$sid.$session.v1"
function Assert-NoInstance {
    $existing = $null
    try { $existing = [Threading.Mutex]::OpenExisting($mutexName) }
    catch [Threading.WaitHandleCannotBeOpenedException] { }
    if ($null -ne $existing) { $existing.Dispose(); throw 'BLOCKED: existing same-user/session AI Relay; no launch, IPC, or exit command sent' }
    $others = @(Get-Process -Name 'ai-relay' -ErrorAction SilentlyContinue | Where-Object { $_.SessionId -eq $session })
    if ($others.Count -gt 0) { throw "BLOCKED: existing ai-relay PID(s) $($others.Id -join ','); left untouched" }
}
Assert-NoInstance
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "Build unavailable: $ExePath. No process launched; this script never runs Cargo." }
$exe = (Get-Item -LiteralPath $ExePath).FullName
$outputRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\target\status-bar-screenshots'))
$output = Join-Path $outputRoot ([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($output)
$temp = Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-status-bar-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temp)
$child = $null; $window = [IntPtr]::Zero; $status = [IntPtr]::Zero
$childHandle = [IntPtr]::Zero; $childStart = $null; $ownedPid = 0
$failure = $null; $cleanupErrors = [Collections.Generic.List[string]]::new()
$uiFailures = [Collections.Generic.List[string]]::new()
$checks = [Collections.Generic.List[object]]::new(); $shots = [Collections.Generic.List[object]]::new()
$overall = [Diagnostics.Stopwatch]::StartNew()
$savedCursor = New-Object StatusBarNative+Point
$haveCursor = [StatusBarNative]::GetCursorPos([ref]$savedCursor)
$previousDpi = [IntPtr]::Zero
function Record([string]$Name, [object]$Value) {
    $checks.Add([pscustomobject]@{ Check=$Name; Value=$Value; Seconds=[Math]::Round($overall.Elapsed.TotalSeconds,2) })
    Write-Output ("CHECK {0}: {1}" -f $Name, ($Value | ConvertTo-Json -Compress -Depth 6))
}
function Assert-Child {
    if ($null -eq $child -or $child.HasExited) { throw 'Owned child missing/exited; refusing automation' }
    [StatusBarNative]::CheckProcess($childHandle,[uint32]$ownedPid)
    if ($child.Id -ne $ownedPid -or $child.Handle -ne $childHandle -or $child.StartTime -ne $childStart -or $child.Path -ine $exe) {
        throw "Child identity changed; refusing automation or termination (PID $($child.Id)/$ownedPid; handle $($child.Handle)/$childHandle; start $($child.StartTime.ToString('o'))/$($childStart.ToString('o')); path '$($child.Path)'/'$exe')"
    }
}
function Control([int]$Id) {
    Assert-Child
    [ProxyUiSmoke]::CheckOwner($window,[uint32]$ownedPid)
    $h = [ProxyUiSmoke]::GetDlgItem($window,$Id)
    [ProxyUiSmoke]::CheckOwner($h,[uint32]$ownedPid)
    return $h
}
function Wait-State([string]$Name, [scriptblock]$Test) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ($timer.Elapsed.TotalSeconds -lt 15 -and $overall.Elapsed.TotalSeconds -lt 240) {
        Assert-Child
        if (& $Test) { return }
        Start-Sleep -Milliseconds 80
    }
    throw "Bounded wait expired: $Name"
}
function Assert-Geometry([string]$Label) {
    Assert-Child
    if ([ProxyUiSmoke]::Class($status) -ine 'Static' -or -not [ProxyUiSmoke]::IsWindowVisible($status)) { throw "$Label status is not a visible native Static" }
    if ([StatusBarNative]::GetParent($status) -ne $window) { throw 'Status parent changed' }
    $client = [StatusBarNative]::Client($window,[uint32]$ownedPid)
    $rect = [StatusBarNative]::InParent($status,$window,[uint32]$ownedPid)
    $dpi = [ClarityNative]::GetDpiForWindow($window)
    if ($dpi -eq 0) { throw 'Window DPI unavailable' }
    $height = [int][Math]::Floor((56 * [double]$dpi + 48) / 96)
    if ($rect.Left -ne 0 -or $rect.Right -ne $client.Right -or $rect.Bottom -ne $client.Bottom -or $rect.Height -ne $height) {
        throw "$Label status geometry $rect vs parent $client; expected height $height physical px (56 DIP), DPI $dpi"
    }
    # Check every visible direct sibling, including unfamiliar native classes.
    $siblings = [StatusBarNative]::VisibleSiblings($window,$status,[uint32]$ownedPid)
    foreach ($sibling in $siblings) {
        $r = [StatusBarNative]::InParent($sibling,$window,[uint32]$ownedPid)
        if ($r.Left -lt $rect.Right -and $r.Right -gt $rect.Left -and $r.Top -lt $rect.Bottom -and $r.Bottom -gt $rect.Top) {
            throw "$Label visible sibling overlaps status: class=$([ProxyUiSmoke]::Class($sibling)) ID=$([ClarityNative]::GetDlgCtrlID($sibling)) rect=$r status=$rect"
        }
    }
    Record "$Label geometry/no visible sibling overlap" @{ Status="$rect"; Client="$client"; DPI=$dpi; VisibleSiblings=$siblings.Length }
}
function Select-Page([int]$Index, [int]$Expected, [string]$Label) {
    $tab = Control 2100
    [ClarityNative]::ClickTab($tab,[uint32]$ownedPid,$Index)
    Wait-State "$Label tab $Index selection + page" {
        ([ClarityNative]::Message($tab,[uint32]$ownedPid,0x130B,0,0) -eq $Index) -and [ProxyUiSmoke]::IsWindowVisible((Control $Expected))
    }
    foreach ($id in @(2102,2332,2200,2001)) {
        if ($id -ne $Expected -and [ProxyUiSmoke]::IsWindowVisible((Control $id))) { throw "$Label outgoing page control $id remains visible" }
    }
    Assert-Geometry "$Label tab $Index (page $Expected)"
}
function Assert-Text([string]$Value) {
    if ($overall.Elapsed.TotalSeconds -ge 240) { throw 'Overall 240-second test budget exceeded' }
    Assert-Child
    [ProxyUiSmoke]::CheckOwner($status,[uint32]$ownedPid)
    [ProxyUiSmoke]::SetText($status,[uint32]$ownedPid,$Value)
    $actual = [ClarityNative]::Text($status,[uint32]$ownedPid)
    if (-not [String]::Equals($actual,$Value,[StringComparison]::Ordinal)) { throw "Native WM_GETTEXT changed/truncated text: expected $($Value.Length), actual $($actual.Length) UTF-16 units" }
    if ([ClarityNative]::Message($status,[uint32]$ownedPid,0xE,0,0) -ne $Value.Length) { throw 'WM_GETTEXTLENGTH differs from full original text' }
}
function Stop-Gui {
    Assert-Child
    [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,1002)
    if (-not $child.WaitForExit(38000)) { throw 'Owned child clean exit exceeded 38 seconds' }
    if ($child.ExitCode -ne 0) { throw "Owned child exit code $($child.ExitCode)" }
    Record 'owned child clean exit via command 1002' $ownedPid
    $child.Dispose(); $script:child=$null; $script:window=[IntPtr]::Zero; $script:status=[IntPtr]::Zero
}
try {
    $previousDpi = [StatusBarNative]::EnterDpi()
    foreach ($theme in @('light','dark')) {
        Assert-NoInstance
        # No mock, socket, HTTP request, real upstream or secret. Config and all
        # generated history/log/UI preference files live only in this unique temp.
        $toml = "schema_version = 1`n[proxy]`nauto_start = false`nforward_enabled = false`nconnect_enabled = false`n[ui]`ntheme = '$theme'`nremember_window_geometry = false`n[system]`nlogin_auto_start = false`n"
        [IO.File]::WriteAllText((Join-Path $temp 'config.toml'),$toml,[Text.UTF8Encoding]::new($false))
        Assert-NoInstance
        $child = Start-Process -FilePath $exe -WorkingDirectory $temp -ArgumentList @('--config-dir',('"'+$temp+'"')) -PassThru
        $childHandle=$child.Handle; $childStart=$child.StartTime; $ownedPid=$child.Id
        $identityTimer=[Diagnostics.Stopwatch]::StartNew()
        do {
            [StatusBarNative]::CheckProcess($childHandle,[uint32]$ownedPid)
            $child.Refresh()
            if ($child.Path -ieq $exe) { break }
            if ($identityTimer.Elapsed.TotalSeconds -ge 5) { throw 'Owned child path metadata unavailable after five seconds; no automation sent' }
            Start-Sleep -Milliseconds 80
        } while ($true)
        Wait-State "$theme own main window" { $script:window=[ProxyUiSmoke]::Find([uint32]$ownedPid); $window -ne [IntPtr]::Zero }
        Wait-State "$theme private config initialized" {
            # Main HWND is enumerated before run() finishes creating its children.
            $save = [ProxyUiSmoke]::GetDlgItem($window,2001)
            if ($save -eq [IntPtr]::Zero) { return $false }
            [ProxyUiSmoke]::CheckOwner($save,[uint32]$ownedPid)
            [ProxyUiSmoke]::IsWindowEnabled($save)
        }
        Wait-State "$theme native status initialized" {
            $found = @([ProxyUiSmoke]::Children($window,'Static') | Where-Object {
                [StatusBarNative]::GetParent($_) -eq $window -and [ClarityNative]::Text($_,[uint32]$ownedPid).StartsWith('代理状态：')
            })
            if ($found.Count -eq 1) { $script:status=$found[0]; return $true }
            return $false
        }
        $original = [ClarityNative]::Text($status,[uint32]$ownedPid)
        if ($original -match '运行中|启动中') { throw 'Proxy unexpectedly started despite private auto_start=false' }
        Record "$theme pinned child/native Static" @{ PID=$ownedPid; Start=$childStart.ToString('o'); InitialText=$original }
        $resizeChanges=0
        foreach ($size in @('initial','large','small')) {
            if ($size -ne 'initial') {
                $before = [StatusBarNative]::Outer($window,[uint32]$ownedPid)
                $requested = [StatusBarNative]::Resize($window,[uint32]$ownedPid,($size -eq 'small'))
                Wait-State "$theme $size resize completed" {
                    $r=[StatusBarNative]::Outer($window,[uint32]$ownedPid)
                    $r.Width -eq $requested.Width -and $r.Height -eq $requested.Height
                }
                $after = [StatusBarNative]::Outer($window,[uint32]$ownedPid)
                if ($before.Width -eq $after.Width -and $before.Height -eq $after.Height) {
                    Record "$theme $size resize clamped by monitor" "$after"
                } else { $resizeChanges++ }
            }
            Select-Page 0 2102 "$theme $size"
            Select-Page 1 2332 "$theme $size"
            Select-Page 2 2200 "$theme $size"
            Select-Page 3 2001 "$theme $size"
        }
        if ($resizeChanges -eq 0) { throw 'BLOCKED: monitor/minimum dimensions prevented any real resize; resize coverage cannot pass' }
        Record "$theme actual window dimension changes" $resizeChanges
        Select-Page 0 2102 "$theme text checks"
        foreach ($value in @('', '状态 & A&&B', "代理状态：已停止`r`n第一行 & 原样`r`n第二行`n第三行`r`n")) { Assert-Text $value }
        $long = "代理状态：已停止 & 原生 STATIC`r`n" + ('长文本 a&b && C:\local\a&b — 状态栏完整文本。 ' * 120) + "`r`n末行 & 保留 CRLF`n尾行"
        Assert-Text $long
        Record "$theme WM_GETTEXT exact Unicode/ampersands/CRLF/multiline/long text" $long.Length
        $tips = @([StatusBarNative]::Tooltips($status,[uint32]$ownedPid))
        if ($tips.Count -ne 1) { throw "Expected one child-owned native tooltips_class32, found $($tips.Count)" }
        $tip=$tips[0]
        if ([ClarityNative]::Message($tip,[uint32]$ownedPid,0x40D,0,0) -ne 1) { throw 'Status tooltip does not contain exactly one native tool' }
        Record "$theme tooltip full native TTM_GETTEXTW text" ([StatusBarNative]::VerifyTooltipPointer($childHandle,[uint32]$ownedPid,$tip,$status,$long))
        [StatusBarNative]::Hover($window,$status,[uint32]$ownedPid)
        try {
            Wait-State "$theme native tooltip visible" { [ProxyUiSmoke]::IsWindowVisible($tip) }
            Record "$theme native hover tooltip visible (full text already verified by TTM_GETTEXTW)" $true
        } catch {
            $uiFailures.Add("$theme tooltip hover: $($_.Exception.Message)")
            Record "$theme native hover tooltip FAILED (continuing other checks)" $_.Exception.Message
        }
        [ClarityNative]::Message($tip,[uint32]$ownedPid,0x41C,0,0) | Out-Null # TTM_POP, scalar only
        $gdi = [Collections.Generic.List[uint32]]::new()
        for ($batch=0; $batch -lt 5; $batch++) {
            for ($i=0; $i -lt 80; $i++) {
                Assert-Text ("代理状态：已停止 & $batch/$i`r`n" + ('资源回归 A&B ' * (1 + $i % 12)))
                # Force each invalidated surface through its own paint path;
                # otherwise coalesced paints could conceal a per-paint GDI leak.
                [ClarityNative]::Message($status,[uint32]$ownedPid,0xF,0,0) | Out-Null
            }
            $gdi.Add([StatusBarNative]::Gdi($childHandle,[uint32]$ownedPid))
        }
        $stable = @($gdi.ToArray() | Select-Object -Skip 1)
        $range = $stable | Measure-Object -Minimum -Maximum
        if (($range.Maximum-$range.Minimum) -gt 2 -or ($stable[-1]-$stable[0]) -gt 1) { throw "Persistent GDI resource growth after warmup: $($gdi -join ',')" }
        Record "$theme GDI after five batches of 80 WM_SETTEXT + paints" @($gdi.ToArray())
        Assert-Text $long
        Record "$theme tooltip TTM_GETTEXTW after repeated replacement" ([StatusBarNative]::VerifyTooltipPointer($childHandle,[uint32]$ownedPid,$tip,$status,$long))
        [ClarityNative]::Message($tip,[uint32]$ownedPid,0x41C,0,0) | Out-Null
        Assert-Text "代理状态：已停止 & A&B`r`n状态栏验证：长消息将省略显示，完整文本保留在原生提示中。"
        Assert-Geometry "$theme before physical status capture"
        Assert-Child
        $shot = [ClarityNative]::Capture($status,[uint32]$ownedPid,(Join-Path $output "$theme-status.png"),$true)
        $client = [StatusBarNative]::Client($status,[uint32]$ownedPid)
        if ($shot.Width -ne $client.Width -or $shot.Height -ne $client.Height) { throw 'DPI clipping: status PNG dimensions differ from physical client' }
        $shots.Add([pscustomobject]@{ Theme=$theme; Shot=$shot; DPI=[ClarityNative]::GetDpiForWindow($status) })
        Record "$theme standalone PrintWindow status screenshot" $shot
        Assert-Text $original
        Stop-Gui
    }
    $light=$shots[0].Shot.MeanLuminance; $dark=$shots[1].Shot.MeanLuminance
    if ($light -lt 150 -or $dark -gt 120 -or ($light-$dark) -lt 60) { throw "Light/dark status rendering not distinct: $light / $dark; high contrast is not covered" }
    Record 'distinct restarted light/dark status rendering' @{ Light=$light; Dark=$dark }
    if ($uiFailures.Count -gt 0) { throw ($uiFailures -join '; ') }
} catch { $failure=$_ }
finally {
    $exited=$null -eq $child
    if ($null -ne $child) {
        try {
            if (-not $child.HasExited) {
                Assert-Child
                if ($window -eq [IntPtr]::Zero) { $window=[ProxyUiSmoke]::Find([uint32]$ownedPid) }
                if ($window -ne [IntPtr]::Zero) {
                    [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,1002)
                    if (-not $child.WaitForExit(38000)) { throw 'Owned child did not exit cleanly within 38 seconds; retaining temp data' }
                    if ($child.ExitCode -ne 0) { throw "Cleanup child exit code $($child.ExitCode)" }
                } else { throw 'Owned child has no main window; no foreign instance or PID will be terminated; retaining temp data' }
            }
            $exited=$child.HasExited
        } catch { $cleanupErrors.Add($_.Exception.Message) }
        finally { $child.Dispose(); $child=$null }
    }
    if ($haveCursor) { [StatusBarNative]::RestoreCursor($savedCursor) }
    if ($previousDpi -ne [IntPtr]::Zero) { [StatusBarNative]::RestoreDpi($previousDpi) }
    if ($exited) {
        for ($attempt=0; $attempt -lt 12; $attempt++) {
            try { [IO.Directory]::Delete($temp,$true); break }
            catch {
                if ($attempt -eq 11) { $cleanupErrors.Add("Temporary data cleanup failed: $($_.Exception.Message)") }
                else { Start-Sleep -Milliseconds 150 }
            }
        }
    } else { $cleanupErrors.Add("Owned child not confirmed exited; temporary data retained at $temp") }
    $report=[pscustomobject]@{
        Passed=($null -eq $failure -and $cleanupErrors.Count -eq 0); Exe=$exe; Checks=@($checks.ToArray()); Screenshots=@($shots.ToArray());
        Failure=$(if ($null -ne $failure) { $failure.ToString() } else { $null }); UiFailures=@($uiFailures.ToArray()); CleanupErrors=@($cleanupErrors.ToArray());
        Limits='Current monitor DPI only; no multi-monitor/high-contrast/physical tray acceptance; no network or Cargo; existing user instances untouched'
    }
    [IO.File]::WriteAllText((Join-Path $output 'results.json'),($report | ConvertTo-Json -Depth 10),[Text.UTF8Encoding]::new($true))
    Write-Output "Status-bar artifacts: $output"
}
if ($null -ne $failure) { throw $failure }
if ($cleanupErrors.Count -gt 0) { throw ($cleanupErrors -join '; ') }
Write-Output 'PASS: status-bar native text, tooltip, four tabs, geometry/resize, GDI, light/dark PrintWindow, clean exits and temporary cleanup'
