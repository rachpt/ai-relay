# Windows PowerShell 5.1 x64；交互式 Windows 10/11 桌面。仅本脚本的子进程。
# 不运行 Cargo，不访问网络，不启动代理，不保存设置，不执行旧脚本 body。
# 用法：powershell.exe -NoProfile -File tools\smoke-date-picker-ui.ps1 -ValidateOnly
# 构建后去掉 -ValidateOnly；可用 -ExePath 指定绝对路径。
# 取消日期左侧复选框 = 未设置：开始无下限，结束无上限；结束日期不含当天。
[CmdletBinding()]
param(
    [string]$ExePath,
    [switch]$ValidateOnly
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([IntPtr]::Size -ne 8) { throw 'Use 64-bit powershell.exe -NoProfile' }
if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path $PSScriptRoot '..\target\date-picker-validation\target\debug\ai-relay.exe'
}

# 提取声明而非 dot-source/Invoke-Expression。modern 自身也不执行。
# 与现有 helper 一起编译：Windows PS 5.1 不能引用另一 Add-Type 内存 DLL。
$parts = @(
    @{ File='smoke-proxy-ui.ps1'; Pattern="(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@" },
    @{ File='smoke-ui-clarity.ps1'; Pattern="(?s)Add-Type -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll -TypeDefinition @'\r?\n(.*?)\r?\n'@" },
    @{ File='smoke-status-bar-ui.ps1'; Pattern='(?s)\$statusDeclaration = @''\r?\n(.*?)\r?\n''@' },
    @{ File='smoke-modern-ui.ps1'; Pattern='(?s)\$declarations\.Add\(@''\r?\n(.*?)\r?\n''@\)' }
)
$declarations = [Collections.Generic.List[string]]::new()
foreach ($part in $parts) {
    $source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot $part.File))
    $matches = [regex]::Matches($source,$part.Pattern)
    if ($matches.Count -ne 1) { throw "Expected one declaration in $($part.File); no older script executed" }
    $declarations.Add($matches[0].Groups[1].Value)
}
foreach ($name in @('ProxyUiSmoke','ProxyUiMock','ClarityNative','StatusBarNative','ModernNative','DatePickerNative')) {
    if ($name -as [type]) { throw "Helper $name already loaded; use a fresh powershell.exe -NoProfile" }
}
$declarations.Add(@'
using System;
using System.Globalization;
using System.Runtime.InteropServices;
public static class DatePickerNative {
    [StructLayout(LayoutKind.Sequential)] public struct PickerInfo {
        public uint Size;
        public StatusBarNative.Rect Check;
        public uint CheckState;
        public StatusBarNative.Rect Button;
        public uint ButtonState;
        public IntPtr Edit,UpDown,DropDown;
    }
    [StructLayout(LayoutKind.Sequential)] struct GuiInfo {
        public uint Size,Flags;
        public IntPtr Active,Focus,Capture,MenuOwner,MoveSize,Caret;
        public StatusBarNative.Rect CaretRect;
    }
    [StructLayout(LayoutKind.Sequential)] struct HighContrastInfo {
        public uint Size,Flags; public IntPtr Scheme;
    }
    public sealed class Civil {
        public long State; public string ISO;
        public override string ToString() { return State+":"+ISO; }
    }
    [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr OpenProcess(uint access,bool inherit,uint pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr VirtualAllocEx(IntPtr process,IntPtr address,UIntPtr size,uint kind,uint protection);
    [DllImport("kernel32.dll")] static extern bool VirtualFreeEx(IntPtr process,IntPtr address,UIntPtr size,uint kind);
    [DllImport("kernel32.dll")] static extern bool WriteProcessMemory(IntPtr process,IntPtr address,byte[] bytes,UIntPtr size,out UIntPtr count);
    [DllImport("kernel32.dll")] static extern bool ReadProcessMemory(IntPtr process,IntPtr address,byte[] bytes,UIntPtr size,out UIntPtr count);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr window,uint message,UIntPtr w,IntPtr l,uint flags,uint timeout,out UIntPtr result);
    [DllImport("user32.dll",CharSet=CharSet.Unicode,EntryPoint="SendMessageTimeoutW")] static extern IntPtr SendText(IntPtr window,uint message,UIntPtr w,string l,uint flags,uint timeout,out UIntPtr result);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window,out uint pid);
    [DllImport("user32.dll",SetLastError=true)] static extern bool PostMessageW(IntPtr window,uint message,UIntPtr w,IntPtr l);
    [DllImport("user32.dll")] static extern uint MapVirtualKeyW(uint key,uint kind);
    [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint thread,ref GuiInfo info);
    [DllImport("user32.dll")] static extern bool IsChild(IntPtr parent,IntPtr child);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern bool SystemParametersInfoW(uint action,uint parameter,ref HighContrastInfo value,uint flags);

    // WM_USER 以上的结构指针不由 user32 跨进程封送。所有输入/输出结构均在
    // 已 pin 住的目标子进程 VM 内；绝不把本进程 SYSTEMTIME/RECT 指针发给它。
    // 超时可能仍在执行：保留目标分配至子进程退出，禁止释放仍可能被使用的内存。
    static byte[] ReadNative(IntPtr pinned,uint pid,IntPtr window,uint message,byte[] initial,out long value) {
        StatusBarNative.CheckProcess(pinned,pid); ProxyUiSmoke.CheckOwner(window,pid);
        if(initial.Length<16 || initial.Length>256) throw new Exception("Invalid bounded native buffer");
        IntPtr process=OpenProcess(0x101038,false,pid),remote=IntPtr.Zero;
        bool completed=true;
        if(process==IntPtr.Zero) throw new Exception("Cannot open pinned child VM");
        try {
            StatusBarNative.CheckProcess(process,pid); StatusBarNative.CheckProcess(pinned,pid);
            var size=new UIntPtr((uint)initial.Length);
            remote=VirtualAllocEx(process,IntPtr.Zero,size,0x3000,4);
            if(remote==IntPtr.Zero) throw new Exception("Native target allocation failed");
            UIntPtr count,result;
            if(!WriteProcessMemory(process,remote,initial,size,out count) || count.ToUInt64()!=(ulong)initial.Length)
                throw new Exception("Incomplete target native buffer initialization");
            ProxyUiSmoke.CheckOwner(window,pid); completed=false;
            if(SendMessageTimeoutW(window,message,UIntPtr.Zero,remote,0x22,2000,out result)==IntPtr.Zero)
                throw new Exception("Native query timed out; target buffer retained until child exit");
            completed=true; value=unchecked((long)result.ToUInt64());
            var bytes=new byte[initial.Length];
            if(!ReadProcessMemory(process,remote,bytes,size,out count) || count.ToUInt64()!=(ulong)bytes.Length)
                throw new Exception("Incomplete native target buffer read");
            return bytes;
        } finally {
            bool released=remote==IntPtr.Zero || !completed || VirtualFreeEx(process,remote,UIntPtr.Zero,0x8000);
            CloseHandle(process);
            if(!released) throw new Exception("Native target buffer release failed");
        }
    }
    static string ISO(byte[] bytes,int offset) {
        int year=BitConverter.ToUInt16(bytes,offset),month=BitConverter.ToUInt16(bytes,offset+2),day=BitConverter.ToUInt16(bytes,offset+6);
        return new DateTime(year,month,day).ToString("yyyy-MM-dd",CultureInfo.InvariantCulture);
    }
    public static Civil ReadDate(IntPtr pinned,uint pid,IntPtr window,bool calendar) {
        string expected=calendar?"SysMonthCal32":"SysDateTimePick32";
        ProxyUiSmoke.CheckOwner(window,pid);
        if(ProxyUiSmoke.Class(window)!=expected) throw new Exception("Expected "+expected);
        long state; var bytes=ReadNative(pinned,pid,window,0x1001,new byte[16],out state);
        // DTM_GETSYSTEMTIME: GDT_VALID=0, GDT_NONE=1. MCM_GETCURSEL: BOOL.
        if(calendar) {
            if(state==0) throw new Exception("MCM_GETCURSEL failed");
        } else if(state!=0 && state!=1) throw new Exception("DTM_GETSYSTEMTIME invalid state");
        return new Civil { State=state,ISO=(!calendar && state==1)?"":ISO(bytes,0) };
    }
    public static StatusBarNative.Rect CalendarMinimum(IntPtr pinned,uint pid,IntPtr calendar) {
        ProxyUiSmoke.CheckOwner(calendar,pid);
        if(ProxyUiSmoke.Class(calendar)!="SysMonthCal32") throw new Exception("Expected native month calendar");
        var previous=StatusBarNative.EnterDpi();
        try {
            // MCM_GETMINREQRECT writes a RECT in the pinned target VM. Reading
            // the final themed/font-adjusted requirement must not resize the UI.
            long result; var bytes=ReadNative(pinned,pid,calendar,0x1009,new byte[16],out result);
            var rect=new StatusBarNative.Rect {
                Left=BitConverter.ToInt32(bytes,0),Top=BitConverter.ToInt32(bytes,4),
                Right=BitConverter.ToInt32(bytes,8),Bottom=BitConverter.ToInt32(bytes,12)
            };
            if(result==0 || rect.Width<=0 || rect.Height<=0 || rect.Width>8192 || rect.Height>8192)
                throw new Exception("MCM_GETMINREQRECT failed/invalid physical size");
            return rect;
        } finally { StatusBarNative.RestoreDpi(previous); }
    }
    public static string Range(IntPtr pinned,uint pid,IntPtr picker) {
        long flags; var bytes=ReadNative(pinned,pid,picker,0x1003,new byte[32],out flags);
        if(flags!=3) throw new Exception("DTM_GETRANGE must have both limits");
        return ISO(bytes,0)+".."+ISO(bytes,16);
    }
    public static PickerInfo Info(IntPtr pinned,uint pid,IntPtr picker) {
        int size=Marshal.SizeOf(typeof(PickerInfo));
        if(IntPtr.Size!=8 || size!=72) throw new Exception("Unexpected x64 DATETIMEPICKERINFO layout");
        var initial=new byte[size]; Array.Copy(BitConverter.GetBytes(size),initial,4);
        long result; var bytes=ReadNative(pinned,pid,picker,0x100E,initial,out result);
        // DTM_GETDATETIMEPICKERINFO has no documented return value. Inspect output.
        IntPtr local=Marshal.AllocHGlobal(size);
        try {
            Marshal.Copy(bytes,0,local,size);
            var info=(PickerInfo)Marshal.PtrToStructure(local,typeof(PickerInfo));
            if(info.Size!=size || info.Check.Width<=0 || info.Button.Width<=0) throw new Exception("Invalid native picker rectangles");
            return info;
        } finally { Marshal.FreeHGlobal(local); }
    }
    public static bool SetISO(IntPtr pinned,uint pid,IntPtr picker,string text) {
        StatusBarNative.CheckProcess(pinned,pid); ProxyUiSmoke.CheckOwner(picker,pid);
        // WM_SETTEXT < WM_USER: string marshaling is performed by user32.
        UIntPtr result;
        if(SendText(picker,0xC,UIntPtr.Zero,text,0x22,2000,out result)==IntPtr.Zero) throw new Exception("WM_SETTEXT timeout");
        return result!=UIntPtr.Zero;
    }
    public static void MousePart(IntPtr pinned,uint pid,IntPtr picker,bool button) {
        var previous=StatusBarNative.EnterDpi();
        try {
            var info=Info(pinned,pid,picker); var r=button?info.Button:info.Check;
            ModernNative.Mouse(picker,pid,(r.Left+r.Right)/2,(r.Top+r.Bottom)/2);
        } finally { StatusBarNative.RestoreDpi(previous); }
    }
    public static bool Focused(IntPtr window,uint pid) {
        ProxyUiSmoke.CheckOwner(window,pid); uint actual;
        uint thread=GetWindowThreadProcessId(window,out actual);
        var info=new GuiInfo { Size=(uint)Marshal.SizeOf(typeof(GuiInfo)) };
        if(actual!=pid || !GetGUIThreadInfo(thread,ref info)) throw new Exception("Cannot inspect target focus");
        return info.Focus==window || IsChild(window,info.Focus);
    }
    public static bool HighContrast() {
        var info=new HighContrastInfo { Size=(uint)Marshal.SizeOf(typeof(HighContrastInfo)) };
        if(!SystemParametersInfoW(0x42,info.Size,ref info,0)) throw new Exception("Cannot inspect high contrast");
        return (info.Flags&1)!=0;
    }
    public static void Key(IntPtr pinned,uint pid,IntPtr window,uint key) {
        StatusBarNative.CheckProcess(pinned,pid); ProxyUiSmoke.CheckOwner(window,pid);
        if(!ProxyUiSmoke.IsWindowVisible(window) || !ProxyUiSmoke.IsWindowEnabled(window)) throw new Exception("Hidden/disabled keyboard target");
        // HWND-local normal native key events. No SendKeys, SendInput, fake
        // WM_NOTIFY, MCM_SETCURSEL or DTM_SETSYSTEMTIME to manufacture selection.
        uint bits=1u|(MapVirtualKeyW(key,0)<<16);
        if(key>=0x21 && key<=0x28) bits|=0x01000000; // Extended navigation keys.
        if(!PostMessageW(window,0x100,new UIntPtr(key),new IntPtr(unchecked((int)bits))) ||
           !PostMessageW(window,0x101,new UIntPtr(key),new IntPtr(unchecked((int)(bits|0xC0000000u)))))
            throw new Exception("Cannot post HWND-local native key pair");
    }
    public static double Luma(long color) {
        return .2126*(color&255)+.7152*((color>>8)&255)+.0722*((color>>16)&255);
    }
}
'@)
$joined = $declarations -join "`n"
$usingPattern = '(?m)^using [^;\r\n]+;\r?$'
$usings = @([regex]::Matches($joined,$usingPattern) | ForEach-Object { $_.Value.Trim() } | Select-Object -Unique)
$combined = ($usings -join "`n") + "`n" + [regex]::Replace($joined,$usingPattern,'')
Add-Type -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll,Accessibility.dll -TypeDefinition $combined
if ($ValidateOnly) {
    Write-Output 'PASS: PowerShell parse/declaration extraction and combined C# compilation only; no EXE, GUI, network or acceptance tests run'
    return
}

$session = [Diagnostics.Process]::GetCurrentProcess().SessionId
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$mutexName = "Local\AIRelay.$sid.$session.v1"
$child=$null; $childHandle=[IntPtr]::Zero; $childStart=$null; $ownedPid=0
$window=[IntPtr]::Zero; $temp=$null; $config=$null; $configBytes=$null; $failure=$null
$checks=[Collections.Generic.List[object]]::new()
$shots=[Collections.Generic.List[object]]::new()
$cleanupErrors=[Collections.Generic.List[string]]::new()
$overall=[Diagnostics.Stopwatch]::StartNew()
$stage='preflight'; $previousDpi=[IntPtr]::Zero
function Assert-NoOtherProcess {
    $others=@(Get-Process -Name 'ai-relay' -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $ownedPid })
    if ($others.Count -gt 0) { throw "BLOCKED: other ai-relay PID(s) $($others.Id -join ','); stopped automation, no input/IPC/kill to them" }
}
function Assert-NoInstance {
    $existing=$null
    try { $existing=[Threading.Mutex]::OpenExisting($mutexName) }
    catch [Threading.WaitHandleCannotBeOpenedException] { }
    if ($null -ne $existing) { $existing.Dispose(); throw 'BLOCKED: existing same-user/session mutex; no launch or IPC sent' }
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
    if ($overall.Elapsed.TotalSeconds -ge 240) { throw "Overall 240-second budget exceeded at $stage" }
}
function Record([string]$Name,[object]$Value) {
    $checks.Add([pscustomobject]@{ Check=$Name; Value=$Value; Seconds=[Math]::Round($overall.Elapsed.TotalSeconds,2) })
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
    while ($timer.Elapsed.TotalSeconds -lt 12) {
        Assert-Child
        if (& $Test) { return }
        Start-Sleep -Milliseconds 40
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
        if (-not $values.ContainsKey($key)) { throw 'Missing history operation state 2354; build current source first' }
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
    if ((Operation).rejected -ne 0) { throw "Unexpected rejected command: $(Text 2354) / $(Text 2325)" }
    if ([ProxyUiSmoke]::Dialog([uint32]$ownedPid) -ne [IntPtr]::Zero) { throw 'Unexpected child modal dialog; no answer automatically sent' }
}
function Segment([int]$Id,[int]$Index) {
    Wait-Idle
    $h=Control $Id
    if ([ProxyUiSmoke]::Class($h) -ne 'SysTabControl32') { throw "ID $Id is not native segmented Tab" }
    [ModernNative]::ClickTab($childHandle,[uint32]$ownedPid,$h,$Index)
    Wait-Idle
    if ((Message $Id 0x130B) -ne $Index -or ($Id -ne 2100 -and (Message $Id 0x147) -ne $Index)) { throw "Selection failed: $Id/$Index" }
}
function Page([int]$Index,[int]$Expected) {
    Segment 2100 $Index
    Wait-State "page $Index control $Expected" { Visible $Expected }
}
function Click([int]$Id) {
    Wait-Idle
    [ModernNative]::Click((Control $Id),[uint32]$ownedPid)
    Wait-Idle
}
function Assert-Date([int]$Id,[string]$Expected) {
    $h=Control $Id
    $text=Text $Id
    $raw=[DatePickerNative]::ReadDate($childHandle,[uint32]$ownedPid,$h,$false)
    $length=Message $Id 0xE
    if ($text -cne $Expected -or $raw.ISO -cne $Expected -or $length -ne $Expected.Length -or
        ($Expected.Length -eq 0 -and $raw.State -ne 1) -or ($Expected.Length -ne 0 -and $raw.State -ne 0)) {
        throw "ID $Id expected '$Expected'; bridge='$text', length=$length, native=$raw"
    }
}
function Set-Date([int]$Id,[string]$Value,[bool]$Accept=$true) {
    Wait-Idle
    $old=Text $Id
    $accepted=[DatePickerNative]::SetISO($childHandle,[uint32]$ownedPid,(Control $Id),$Value)
    if ($accepted -ne $Accept) { throw "WM_SETTEXT ID $Id '$Value': result=$accepted expected=$Accept" }
    $expected=if ($Accept) { $Value } else { $old }
    Assert-Date $Id $expected
}
function Assert-ConfigUnchanged {
    if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($config)) -cne $configBytes) { throw 'Date/theme/query/navigation wrote config without Save' }
    if ([ProxyUiSmoke]::IsWindowEnabled((Control 2003))) { throw 'Proxy unexpectedly running; test must not start networking' }
}
function Calendar([int]$Id) {
    $h=[IntPtr](Message $Id 0x1008)
    if ($h -ne [IntPtr]::Zero) {
        [ProxyUiSmoke]::CheckOwner($h,[uint32]$ownedPid)
        if ([ProxyUiSmoke]::Class($h) -ne 'SysMonthCal32') { throw 'DTM_GETMONTHCAL returned non-native calendar' }
    }
    return $h
}
function Assert-CalendarSize([int]$Id,[IntPtr]$Handle) {
    Assert-Child
    $minimum=[DatePickerNative]::CalendarMinimum($childHandle,[uint32]$ownedPid,$Handle)
    $client=[StatusBarNative]::Client($Handle,[uint32]$ownedPid)
    $outer=[StatusBarNative]::Outer($Handle,[uint32]$ownedPid)
    Record "picker $Id popup client >= MCM_GETMINREQRECT" @{
        DPI=[ClarityNative]::GetDpiForWindow($Handle); Client="$client"; Minimum="$minimum"; Outer="$outer"
    }
    if ($client.Width -lt $minimum.Width -or $client.Height -lt $minimum.Height) {
        throw "Picker $Id popup clipped: physical client=$client; MCM_GETMINREQRECT=$minimum; outer=$outer"
    }
}
function Open-Calendar([int]$Id) {
    Wait-Idle
    if ((Calendar $Id) -ne [IntPtr]::Zero) { throw 'Calendar already open before native button click' }
    [DatePickerNative]::MousePart($childHandle,[uint32]$ownedPid,(Control $Id),$true)
    Wait-State "picker $Id actual native popup" { (Calendar $Id) -ne [IntPtr]::Zero -and [ProxyUiSmoke]::IsWindowVisible((Calendar $Id)) }
    $calendar=Calendar $Id
    Assert-CalendarSize $Id $calendar
    return $calendar
}
function Close-Calendar([int]$Id) {
    $calendar=Calendar $Id
    if ($calendar -ne [IntPtr]::Zero) {
        [DatePickerNative]::Key($childHandle,[uint32]$ownedPid,$calendar,0x1B)
        Fence
        # Some comctl32 versions route Escape through the picker/dialog loop.
        # A documented scalar close is safe; it never sets the selected date.
        if ((Calendar $Id) -ne [IntPtr]::Zero) { Message $Id 0x100D | Out-Null }
        Wait-State "picker $Id popup closed" { (Calendar $Id) -eq [IntPtr]::Zero }
    }
}
function Snapshot([string]$Name,[IntPtr]$Handle,[switch]$Outer) {
    Assert-Child
    if (-not [ProxyUiSmoke]::IsWindowVisible($Handle)) { throw "Hidden screenshot target: $Name" }
    $r=New-Object StatusBarNative+Rect
    $shot=[ModernNative]::Capture($Handle,[uint32]$ownedPid,(Join-Path $output "$Name.png"),(-not $Outer),$r)
    $actual=if ($Outer) { [StatusBarNative]::Outer($Handle,[uint32]$ownedPid) } else { [StatusBarNative]::Client($Handle,[uint32]$ownedPid) }
    if ($shot.Width -ne $actual.Width -or $shot.Height -ne $actual.Height) { throw 'Screenshot is not actual physical window size' }
    $shots.Add([pscustomobject]@{ Name=$Name; DPI=[ClarityNative]::GetDpiForWindow($Handle); PhysicalRect="$actual"; Shot=$shot })
    Record "physical PNG $Name" $shot
    return $shot
}
function Assert-Layout {
    $dpi=[ClarityNative]::GetDpiForWindow($window)
    if ($dpi -lt 96) { throw 'Invalid target DPI' }
    $client=[StatusBarNative]::Client($window,[uint32]$ownedPid)
    foreach ($pair in @(@(2302,24),@(2303,372))) {
        $r=[StatusBarNative]::InParent((Control $pair[0]),$window,[uint32]$ownedPid)
        # History stretches each date field with the page width; only y/height
        # are fixed DIP. Match the production integer arithmetic, not a 760-DIP window.
        $margin = [int][Math]::Floor((24*$dpi+48)/96)
        $usable = $client.Width - [int][Math]::Floor((48*$dpi+48)/96)
        $expected = @(
            ($margin + [int][Math]::Truncate($usable*($pair[1]-24)/696)),
            [int][Math]::Floor((136*$dpi+48)/96),
            [int][Math]::Truncate($usable*336/696),
            [int][Math]::Floor((28*$dpi+48)/96)
        )
        if ([Math]::Abs($r.Left-$expected[0]) -gt 1 -or [Math]::Abs($r.Top-$expected[1]) -gt 1 -or
            [Math]::Abs($r.Width-$expected[2]) -gt 1 -or [Math]::Abs($r.Height-$expected[3]) -gt 1 -or
            $r.Right -gt $client.Width -or $r.Bottom -gt $client.Height) { throw "Picker $($pair[0]) clipped/wrong layout: $r at DPI $dpi" }
        Record "picker $($pair[0]) native geometry" @{ DPI=$dpi; Physical="$r"; ExpectedDIP="$($pair[1]),136 336x28" }
    }
    $a=[StatusBarNative]::InParent((Control 2302),$window,[uint32]$ownedPid)
    $b=[StatusBarNative]::InParent((Control 2303),$window,[uint32]$ownedPid)
    if ($a.Right -ge $b.Left -or $a.Top -ne $b.Top) { throw 'Date pickers overlap/not on same row' }
}
function Preview-Theme([int]$Index) {
    Page 3 2039
    if ([ProxyUiSmoke]::Class((Control 2039)) -ne 'ComboBox') { throw 'Settings category is not native ComboBox' }
    Message 2039 0x14E 5 | Out-Null
    [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,[uint32](2039 -bor 0x10000))
    Wait-Idle
    Wait-State 'appearance category' { Visible 2030 }
    Segment 2030 $Index
    Page 1 2300
    Assert-ConfigUnchanged
}

# Any existing same-user/session mutex OR ai-relay PID blocks launch, even if its
# EXE differs. No CLI show/quit/stop can accidentally target a user instance.
Assert-NoInstance
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
    throw "Build unavailable: $([IO.Path]::GetFullPath($ExePath)). Build later and rerun; this script never runs Cargo."
}
$exe=(Get-Item -LiteralPath $ExePath).FullName
$outputRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\target\date-picker-screenshots'))
$output=Join-Path $outputRoot ([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')+'-'+[Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($output)
try {
    $previousDpi=[StatusBarNative]::EnterDpi()
    Assert-NoInstance
    $temp=Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-date-picker-'+[Guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($temp)
    $config=Join-Path $temp 'config.toml'
    # Deliberately unusable loopback upstream; no socket/mock/HTTP request needed.
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
        if ($identityTimer.Elapsed.TotalSeconds -ge 5) { throw 'Child executable metadata unavailable; no GUI input sent' }
        Start-Sleep -Milliseconds 40
    } while ($true)
    Wait-State 'own main HWND' { $script:window=[ProxyUiSmoke]::Find([uint32]$ownedPid); $window -ne [IntPtr]::Zero }
    Wait-State 'private config loaded; proxy stopped' {
        $h=[ProxyUiSmoke]::GetDlgItem($window,2002)
        $h -ne [IntPtr]::Zero -and [ProxyUiSmoke]::IsWindowEnabled($h) -and -not [ProxyUiSmoke]::IsWindowEnabled((Control 2003))
    }
    Assert-ConfigUnchanged
    Record 'pinned child; no network or real credentials' @{ PID=$ownedPid; Start=$childStart.ToString('o'); Exe=$exe }
    [StatusBarNative]::Resize($window,[uint32]$ownedPid,$false) | Out-Null
    Page 1 2300
    Wait-Idle
    $stage='native identity, initial empty and custom visibility'
    foreach ($id in @(2302,2303)) {
        if ([ProxyUiSmoke]::Class((Control $id)) -ne 'SysDateTimePick32') { throw "ID $id must be native SysDateTimePick32" }
        if (([ProxyUiSmoke]::GetWindowLongW((Control $id),-16) -band 2) -eq 0) { throw "ID $id missing DTS_SHOWNONE" }
        Assert-Date $id ''
        $range=[DatePickerNative]::Range($childHandle,[uint32]$ownedPid,(Control $id))
        if ($range -cne '1970-01-01..9999-12-31') { throw "ID $id native range mismatch: $range" }
        Record "native $id initial GDT_NONE + ISO empty + range" $range
    }
    foreach ($index in @(0,1,2,3,4)) {
        Segment 2301 $index
        if ((Visible 2302) -ne ($index -eq 4) -or (Visible 2303) -ne ($index -eq 4)) { throw "Custom visibility failed for preset $index" }
    }
    Assert-Layout
    $stage='WM_SETTEXT valid, leap, bounds, rejection preservation and clear'
    foreach ($id in @(2302,2303)) {
        foreach ($valid in @('2024-02-29','2000-02-29','1970-01-01','9999-12-31','2024-02-15')) { Set-Date $id $valid }
        foreach ($invalid in @('2023-02-29','2100-02-29','2024-04-31','1969-12-31','10000-01-01','2024-13-01','2024-00-01','2024-01-00','2024-2-09','2024-02-09 ',' 2024-02-09','not-a-date','２０２４-０２-１５')) {
            Set-Date $id $invalid $false
        }
        Set-Date $id ''
        Record "picker $id strict bridge/native state, leap/range/invalid preservation/clear" $true
    }
    $stage='native DTS_SHOWNONE checkbox clears and enables'
    foreach ($id in @(2302,2303)) {
        Set-Date $id '2024-02-15'
        [DatePickerNative]::MousePart($childHandle,[uint32]$ownedPid,(Control $id),$false)
        Fence
        Assert-Date $id ''
        [DatePickerNative]::MousePart($childHandle,[uint32]$ownedPid,(Control $id),$false)
        Fence
        if ((Text $id) -eq '') { throw "Native checkbox $id failed to enable a date" }
        Assert-Date $id (Text $id)
    }
    Record 'checkbox unchecked = no lower/upper bound, not a min/max sentinel' $true

    $stage='real month calendar keyboard navigation and selection'
    foreach ($id in @(2302,2303)) {
        Set-Date $id '2024-02-15'
        $calendar=Open-Calendar $id
        $before=[DatePickerNative]::ReadDate($childHandle,[uint32]$ownedPid,$calendar,$true)
        if ($before.ISO -cne '2024-02-15') { throw "Native popup initial selection mismatch: $before" }
        [DatePickerNative]::Key($childHandle,[uint32]$ownedPid,$calendar,0x22) # PageDown: next month
        Wait-State 'native PageDown moved to March' {
            ([DatePickerNative]::ReadDate($childHandle,[uint32]$ownedPid,$calendar,$true)).ISO -ceq '2024-03-15'
        }
        [DatePickerNative]::Key($childHandle,[uint32]$ownedPid,$calendar,0x27) # Right: next day
        Wait-State 'native Right selected March 16' {
            ([DatePickerNative]::ReadDate($childHandle,[uint32]$ownedPid,$calendar,$true)).ISO -ceq '2024-03-16'
        }
        [DatePickerNative]::Key($childHandle,[uint32]$ownedPid,$calendar,0x0D)
        Fence
        if ((Calendar $id) -ne [IntPtr]::Zero) {
            [DatePickerNative]::Key($childHandle,[uint32]$ownedPid,(Control $id),0x0D)
            Fence
        }
        # Native selection must already reach the picker, not just the popup.
        Assert-Date $id '2024-03-16'
        Close-Calendar $id
        Assert-Date $id '2024-03-16'
        Record "picker $id native mouse dropdown/PageDown/Right/Enter updates bridge" @{ Before=$before.ISO; After=(Text $id) }
    }

    $stage='Tab and timezone retain civil dates'
    Set-Date 2302 '2024-02-29'
    Set-Date 2303 '2024-03-02'
    Page 0 2102
    if ((Visible 2302) -or (Visible 2303)) { throw 'Date controls leaked onto overview' }
    Page 2 2228
    Page 1 2300
    Assert-Date 2302 '2024-02-29'
    Assert-Date 2303 '2024-03-02'
    foreach ($zone in @(1,0)) {
        Segment 2333 $zone
        Assert-Date 2302 '2024-02-29'
        Assert-Date 2303 '2024-03-02'
    }
    Record 'page roundtrip and UTC/Local preserve civil dates' $true

    $stage='empty local backend query, observed pending and frozen native dates'
    Click 2320
    if (-not (Text 2325).Contains('匹配 0 请求')) { throw "Empty private history query failed: $(Text 2325)" }
    Assert-Child
    $pending=[ModernNative]::ProbePending($childHandle,[uint32]$ownedPid,$window)
    Wait-Idle
    if (-not $pending.Seen -or $pending.Frozen -notcontains '2302' -or $pending.Frozen -notcontains '2303') { throw 'Native date pending freeze not observed' }
    foreach ($pair in @(@(2301,$pending.Date),@(2333,$pending.Zone),@(2334,$pending.Trend),@(2338,$pending.Table),@(2353,$pending.Export))) {
        if ((Message $pair[0] 0x147) -ne $pair[1]) { throw "Pending changed selection $($pair[0])" }
    }
    Assert-Date 2302 '2024-02-29'
    Assert-Date 2303 '2024-03-02'
    Record 'real pending freezes 2302/2303; dates and selections retained' $pending
    # This no-network fixture cannot prove boundary counts from zero rows.
    Record 'NOT VALIDATED: query start-inclusive/end-exclusive record membership' 'No seeded mock; native ISO dates and real empty query only. Requires local mock records/boundary timestamps for membership acceptance.'
    foreach ($zone in @(1,0)) {
        Segment 2333 $zone
        $zoneName=if ($zone -eq 1) { 'Local' } else { 'Utc' }
        if (-not (Text 2325).Contains("$zoneName/Day")) { throw "Timezone automatic query failed: $(Text 2325)" }
        Assert-Date 2302 '2024-02-29'
        Assert-Date 2303 '2024-03-02'
    }

    $stage='physical light/dark closed and popup evidence'
    $highContrast=[DatePickerNative]::HighContrast()
    Record 'system high contrast (never changed)' $highContrast
    foreach ($theme in @(@(1,'light'),@(2,'dark'))) {
        $name=$theme[1]
        Preview-Theme $theme[0]
        Assert-Date 2302 '2024-02-29'
        Assert-Date 2303 '2024-03-02'
        Assert-Layout
        Segment 2301 4 # Real click moves focus away from date, without a query.
        Snapshot "$name-custom-window" $window -Outer | Out-Null
        foreach ($id in @(2302,2303)) {
            if ([DatePickerNative]::Focused((Control $id),[uint32]$ownedPid)) { throw 'Cannot test unfocused closed appearance while picker focused' }
            $closed=Snapshot "$name-picker-$id-closed" (Control $id)
            if (-not $highContrast) {
                if ($name -eq 'dark' -and ($closed.MeanLuminance -gt 135 -or $closed.BrightTileFraction -gt .08)) { throw 'Dark unfocused picker has a large light background' }
                if ($name -eq 'light' -and $closed.MeanLuminance -lt 150) { throw 'Light closed picker unexpectedly dark' }
            }
            $calendar=Open-Calendar $id
            Snapshot "$name-picker-$id-popup-window" $window -Outer | Out-Null
            $popup=Snapshot "$name-picker-$id-popup-deep" $calendar -Outer
            Assert-CalendarSize $id $calendar
            $colors=@()
            foreach ($part in 0..5) {
                $pickerColor=Message $id 0x1007 ([uint32]$part)
                $calendarColor=[ModernNative]::Message($calendar,[uint32]$ownedPid,0x100B,[uint32]$part,0)
                if ($pickerColor -lt 0 -or $calendarColor -lt 0 -or $pickerColor -ne $calendarColor) { throw "Picker/calendar public color mismatch: $id/$part" }
                $colors+=('{0:X6}' -f $calendarColor)
            }
            $surface=Message $id 0x1007 4
            $ink=Message $id 0x1007 1
            if ([Math]::Abs([DatePickerNative]::Luma($surface)-[DatePickerNative]::Luma($ink)) -lt 60) { throw 'Calendar public text/month surface colors lack contrast' }
            # Keep native visual styles: documented MCSC colors are best effort
            # and need not match rendered pixels. A white popup is an explicit
            # fallback, not a failure; size/paint/integrity checks remain strict.
            Record "$name-picker-$id popup native theme fallback" @{
                MeanLuminance=$popup.MeanLuminance; BrightFraction=$popup.BrightFraction;
                BrightTileFraction=$popup.BrightTileFraction; HighContrast=$highContrast;
                Note='Native popup theme retained; public colors are best effort. A light/white popup is allowed even in dark mode; full dark popup rendering is not required.'
            }
            Record "$name-picker-$id popup documented MCSC colors (best effort; not rendered pixel proof)" $colors
            Close-Calendar $id
            # Focused closed control intentionally uses native editing rendering.
            $focused=[DatePickerNative]::Focused((Control $id),[uint32]$ownedPid)
            Snapshot "$name-picker-$id-after-popup-native-focus" (Control $id) | Out-Null
            Record "$name-picker-$id native focus fallback" @{ Focused=$focused; Note='Focused editing/high contrast/disabled appearance may remain system themed; no claim of fully dark native chrome.' }
            Assert-Date $id (@{2302='2024-02-29';2303='2024-03-02'}[$id])
            Segment 2301 4
        }
        Assert-ConfigUnchanged
    }

    $stage='custom reset hides and clears without replacing applied report'
    $oldSummary=Text 2330; $oldMetadata=Text 2331; $oldRows=Message 2337 0x1004
    $oldSequence=(Operation).sequence
    Click 2317
    if ((Message 2301 0x147) -ne 0 -or (Visible 2302) -or (Visible 2303) -or -not (Text 2325).Contains('筛选草稿已重置')) { throw 'Reset failed to hide custom dates' }
    Assert-Date 2302 ''
    Assert-Date 2303 ''
    if ((Operation).sequence -ne $oldSequence -or (Text 2330) -cne $oldSummary -or (Text 2331) -cne $oldMetadata -or (Message 2337 0x1004) -ne $oldRows) { throw 'Draft reset queried or changed applied report' }
    Record 'reset hides/clears both native dates and preserves applied report' $true
    Assert-ConfigUnchanged
    $stage='clean exact-child exit'
    Assert-Child
    [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,1002)
    if (-not $child.WaitForExit(38000)) { throw 'Owned child exit exceeded 38 seconds; no kill used' }
    if ($child.ExitCode -ne 0) { throw "Owned child exit code $($child.ExitCode)" }
    if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($config)) -cne $configBytes) { throw 'Clean exit wrote unsaved config' }
    Record 'clean exact-child 1002 exit; config bytes unchanged' $ownedPid
} catch {
    $failure=$_
    Write-Warning ("FAIL stage={0}; {1}; {2}" -f $stage,$_.Exception.Message,$_.InvocationInfo.PositionMessage)
} finally {
    $exited=$null -eq $child
    if ($null -ne $child) {
        try {
            if (-not $child.HasExited) {
                # Another instance stops tests immediately, but cleanup may still
                # address ONLY our retained process/window. Never CLI or Kill.
                Assert-Identity
                if ($window -eq [IntPtr]::Zero) { $window=[ProxyUiSmoke]::Find([uint32]$ownedPid) }
                if ($window -ne [IntPtr]::Zero) { [ProxyUiSmoke]::Post($window,[uint32]$ownedPid,0x111,1002) }
                if (-not $child.WaitForExit(38000)) { throw 'Owned child cleanup 1002 exit timed out; process left alive, temp retained; never killed' }
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
        Passed=($null -eq $failure -and $cleanupErrors.Count -eq 0); Exe=$exe; PID=$ownedPid;
        Failure=$detail; CleanupErrors=@($cleanupErrors.ToArray()); Checks=@($checks.ToArray()); Screenshots=@($shots.ToArray());
        Seconds=[Math]::Round($overall.Elapsed.TotalSeconds,2);
        Limits='Current physical monitor DPI only. No multi-monitor/DPI matrix, screen reader, real physical keyboard or full high-contrast acceptance. Native focused editing and themed month-calendar popups may use light system fallback; public popup colors are best effort, not rendered pixel proof. Strict popup minimum size and paint/integrity checks remain required. No mock records: start-inclusive/end-exclusive membership NOT validated. No network, config Save, Startup/firewall mutation, Cargo, global input, other-instance IPC or kill. Pending must be observed, never manufactured.'
    }
    [IO.File]::WriteAllText((Join-Path $output 'results.json'),($report | ConvertTo-Json -Depth 12),[Text.UTF8Encoding]::new($true))
    Write-Output "Date picker artifacts: $output"
}
if ($null -ne $failure) { throw $failure }
if ($cleanupErrors.Count -gt 0) { throw ($cleanupErrors -join '; ') }
Write-Output 'PASS: native date identity/ISO/native VM state, real month navigation/day selection, visibility/reset, Tab/timezone preservation, observed pending freeze, physical light/dark screenshots and unchanged config. Boundary record membership NOT validated (no mock).'
