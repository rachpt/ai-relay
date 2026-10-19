# Windows PowerShell 5.1 x64 / Windows 10、11 交互桌面；不构建，须使用主 agent 已更新的 EXE。
# 仅普通 Launch --config-dir <绝对私有临时目录>；不以 CLI start/重启/伪造 Start 命令代替自动启动。
# 7 组、10 个短 fixture；仅回环 mock/health，无模型、Key、费用或系统设置变更。
# -ValidateOnly 只读取本脚本及两份 helper 声明、解析/编译和纯布局/属性自检；不查询 EXE、
# 不创建/读取运行配置或报告，不查实例，不启动进程/GUI，不调用 Win32 或网络 API。
# 示例（项目根目录）：powershell.exe -NoProfile -File .\tools\smoke-auto-start-ui.ps1 -ValidateOnly
[CmdletBinding()]
param(
    [string]$ExePath,
    [switch]$ValidateOnly,
    [ValidateRange(60,180)][int]$BudgetSeconds = 180
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$clock = [Diagnostics.Stopwatch]::StartNew()
if ([IntPtr]::Size -ne 8) { throw 'Use a fresh x64 powershell.exe -NoProfile process' }
foreach ($name in @('AutoStartUiSmokeBorrowed.Native','AutoStartUiSmokeFixtures.ProxyUiMock','AutoStartUiSmoke.Native')) {
    if ($name -as [type]) { throw 'Auto-start helpers already loaded; use a fresh PowerShell process' }
}

# 只取 C# literal；不 dot-source、不 Invoke-Expression、不执行其他 smoke 的 PowerShell body。
$titleSource = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'smoke-title-menu-ui.ps1'))
$titleDeclarations = [regex]::Matches($titleSource,'(?s)\$nativeSource = @''\r?\n(.*?)\r?\n''@')
$proxySource = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'smoke-proxy-ui.ps1'))
$proxyDeclarations = [regex]::Matches($proxySource,'(?s)Add-Type -TypeDefinition @''\r?\n(.*?)\r?\n''@')
if ($titleDeclarations.Count -ne 1 -or $proxyDeclarations.Count -ne 1) { throw 'Expected one literal C# declaration per approved helper script' }
$titleDeclaration = $titleDeclarations[0].Groups[1].Value
if ([regex]::Matches($titleDeclaration,'namespace TitleMenuSmoke \{').Count -ne 1) { throw 'Title helper namespace contract changed' }
$titleDeclaration = $titleDeclaration.Replace('namespace TitleMenuSmoke {','namespace AutoStartUiSmokeBorrowed {')
# Proxy 的 using 和两个 class 一并装入独立 namespace，避免污染其他 smoke 的全局类型。
$proxyBody=$proxyDeclarations[0].Groups[1].Value
$mockConstructor='this.marker=marker; listener=new TcpListener(IPAddress.Loopback,0); listener.Start();'
if ([regex]::Matches($proxyBody,[regex]::Escape($mockConstructor)).Count -ne 1) { throw 'Mock constructor contract changed' }
# 仅修改当前工具借用的内存副本；Framework socket 默认可继承，不能泄漏给待测子进程。
$proxyBody=$proxyBody.Replace($mockConstructor,'this.marker=marker; listener=new TcpListener(IPAddress.Loopback,0); try { AutoStartUiSmoke.Native.NoInherit(listener); listener.Start(); AutoStartUiSmoke.Native.NoInherit(listener); } catch { listener.Stop(); listener.Server.Dispose(); throw; }')
$proxyDeclaration = "namespace AutoStartUiSmokeFixtures {`n" + $proxyBody + "`n}"
$extraDeclaration = @'
namespace AutoStartUiSmoke {
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using N = AutoStartUiSmokeBorrowed.Native;
public static class Native {
    [StructLayout(LayoutKind.Sequential)] struct Row4 { public uint State,Address,Port,RemoteAddress,RemotePort,Owner; }
    [StructLayout(LayoutKind.Sequential)] struct Row6 {
        [MarshalAs(UnmanagedType.ByValArray,SizeConst=16)] public byte[] Address;
        public uint Scope,Port;
        [MarshalAs(UnmanagedType.ByValArray,SizeConst=16)] public byte[] RemoteAddress;
        public uint RemoteScope,RemotePort,State,Owner;
    }
    [DllImport("iphlpapi.dll")] static extern uint GetExtendedTcpTable(IntPtr table,ref int bytes,[MarshalAs(UnmanagedType.Bool)] bool sorted,int family,int kind,uint reserved);
    [DllImport("kernel32.dll",SetLastError=true)] static extern bool GetHandleInformation(IntPtr handle,out uint flags);
    [DllImport("kernel32.dll",SetLastError=true)] static extern bool SetHandleInformation(IntPtr handle,uint mask,uint flags);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool QueryFullProcessImageNameW(IntPtr process,uint flags,StringBuilder path,ref int length);
    [DllImport("user32.dll")] static extern bool ShowWindowAsync(IntPtr h,int command);
    [DllImport("user32.dll")] static extern int GetWindowLongW(IntPtr h,int index);

    public static string Validate() {
        // 只验证托管布局/属性和常量，不触发 P/Invoke、进程、磁盘或 socket。
        if(IntPtr.Size!=8 || Marshal.SizeOf(typeof(Row4))!=24 || Marshal.SizeOf(typeof(Row6))!=56 ||
           Marshal.OffsetOf(typeof(Row4),"Owner").ToInt32()!=20 || Marshal.OffsetOf(typeof(Row6),"Owner").ToInt32()!=52)
            throw new Exception("Unexpected TCP owner-table ABI");
        foreach(string method in new string[] { "GetExtendedTcpTable","GetHandleInformation","SetHandleInformation","QueryFullProcessImageNameW","ShowWindowAsync","GetWindowLongW" }) {
            var member=typeof(Native).GetMethod(method,BindingFlags.Static|BindingFlags.NonPublic);
            if(member==null || member.GetCustomAttributes(typeof(DllImportAttribute),false).Length!=1)
                throw new Exception("Missing P/Invoke declaration attribute");
        }
        if(!AllowedTarget("/__proxy/health") || !AllowedTarget("/smoke?probe=1") || AllowedTarget("/v1/models") || AllowedTarget("http://example.invalid/"))
            throw new Exception("Probe target allowlist failed");
        if(Inheritable(0) || Inheritable(2) || !Inheritable(1) || !Inheritable(3)) throw new Exception("Handle flag self-test failed");
        var harness=new ListenerRow { Owner=2,State=2,Family=2,Port=12345,Loopback=true };
        if(Released(new ListenerRow[] { harness },1,2,12345) || !Released(new ListenerRow[0],1,2,12345)) throw new Exception("Release wait self-test failed");
        foreach(var invalid in new ListenerRow[] {
            new ListenerRow { Owner=1,Port=12345 }, new ListenerRow { Owner=1,Port=54321 },
            new ListenerRow { Owner=3,Port=12345 }, new ListenerRow { Owner=2,Port=12345,Family=23,Loopback=true }
        }) {
            bool rejected=false;
            try { Released(new ListenerRow[] { invalid },1,2,12345); } catch(InvalidOperationException) { rejected=true; }
            if(!rejected) throw new Exception("Release guard self-test failed");
        }
        return "x64 TCP_OWNER_PID rows 24/56; DllImport/handle flags; release guard; two fixed GET targets";
    }
    static bool Inheritable(uint flags) { return (flags&1)!=0; }
    public static void NoInherit(TcpListener listener) {
        var socket=listener.Server; uint flags;
        try {
            if(!GetHandleInformation(socket.Handle,out flags) || !SetHandleInformation(socket.Handle,1,0) ||
               !GetHandleInformation(socket.Handle,out flags) || Inheritable(flags))
                throw new InvalidOperationException("SOCKET_INHERITANCE_GUARD_FAILED");
        } finally { GC.KeepAlive(socket); }
    }
    public static void Image(IntPtr handle,uint pid,string exe) {
        N.CheckProcess(handle,pid);
        var text=new StringBuilder(32768); int length=text.Capacity;
        if(!QueryFullProcessImageNameW(handle,0,text,ref length) ||
           !String.Equals(Path.GetFullPath(text.ToString()),exe,StringComparison.OrdinalIgnoreCase))
            throw new Exception("PINNED_EXE_MISMATCH");
    }
    public static void Tree(IntPtr main,IntPtr h,uint pid) {
        N.Check(main,pid);
        if(N.Class(main)!="AIRelay.Window.v1") throw new Exception("MAIN_CLASS_MISMATCH");
        for(int i=0;i<64 && h!=IntPtr.Zero;i++,h=N.GetParent(h)) { N.Check(h,pid); if(h==main) return; }
        throw new Exception("HWND_OUTSIDE_OWN_MAIN_TREE");
    }
    public static IntPtr Control(IntPtr main,uint pid,int id) {
        var h=N.FindControl(main,pid,id); Tree(main,h,pid); return h;
    }
    public static string Status(IntPtr main,uint pid) {
        Tree(main,main,pid); string result="";
        foreach(var h in N.Children(main,pid,true)) {
            if(!String.Equals(N.Class(h),"Static",StringComparison.OrdinalIgnoreCase) || !N.IsWindowVisible(h)) continue;
            Tree(main,h,pid); string text=N.Text(h,pid);
            if(!text.StartsWith("代理状态：",StringComparison.Ordinal)) continue;
            if(result.Length!=0) throw new Exception("DUPLICATE_STATUS_CONTROL"); result=text;
        }
        return result;
    }
    public static bool Enabled(IntPtr main,uint pid,int id) {
        var h=Control(main,pid,id);
        if(!String.Equals(N.Class(h),"Button",StringComparison.OrdinalIgnoreCase)) throw new Exception("BUTTON_CLASS_MISMATCH");
        return N.IsWindowEnabled(h);
    }
    public static void Page(IntPtr main,uint pid,int index) {
        if(index!=0 && index!=3) throw new Exception("PAGE_NOT_ALLOWED");
        var tab=Control(main,pid,2100);
        if(N.Class(tab)!="SysTabControl32") throw new Exception("TAB_CLASS_MISMATCH");
        // 原生 Tab 自行在目标进程产生通知；不跨进程传 NMHDR/TCITEM 指针。
        N.Message(tab,pid,0x100,0x24,0);
        for(int i=0;i<index;i++) N.Message(tab,pid,0x100,0x27,0);
    }
    public static bool PageIs(IntPtr main,uint pid,int index) {
        return N.Message(Control(main,pid,2100),pid,0x130B,0,0)==index;
    }
    public static void StartupCategory(IntPtr main,uint pid) {
        var h=Control(main,pid,2039);
        if(N.Class(h)!="SysTabControl32" || !N.IsWindowVisible(h)) throw new Exception("CATEGORY_CLASS_OR_VISIBILITY");
        N.Message(h,pid,0x14E,4,0); // 既有 Choices CB_SETCURSEL 标量兼容路由。
        N.Post(main,pid,0x111,2039u|(1u<<16),h.ToInt64());
    }
    public static bool CategoryIs(IntPtr main,uint pid) {
        return N.Message(Control(main,pid,2039),pid,0x130B,0,0)==4;
    }
    public static bool CheckboxVisible(IntPtr main,uint pid) { return N.IsWindowVisible(Control(main,pid,2065)); }
    public static int Checkbox(IntPtr main,uint pid) {
        var h=Control(main,pid,2065);
        if(!String.Equals(N.Class(h),"Button",StringComparison.OrdinalIgnoreCase) || (GetWindowLongW(h,-16)&15)!=3 ||
           !N.IsWindowVisible(h) || N.Text(h,pid)!="打开软件后自动启动代理（使用已保存配置）")
            throw new Exception("AUTO_START_CHECKBOX_CONTRACT");
        long value=N.Message(h,pid,0xF0,0,0); // BM_GETCHECK；绝不 click/BM_SETCHECK/save。
        if(value!=0 && value!=1) throw new Exception("AUTO_START_CHECKBOX_INDETERMINATE");
        return (int)value;
    }
    public static void Command(IntPtr main,uint pid,int id) {
        Tree(main,main,pid);
        if(id!=2103 && id!=1002) throw new Exception("COMMAND_NOT_ALLOWED");
        N.Post(main,pid,0x111,(uint)id,0); // 只用自有主窗口的 Stop / 正常 Exit 路由。
    }
    public static void Hide(IntPtr main,uint pid) { Tree(main,main,pid); if(!ShowWindowAsync(main,0)) throw new Exception("HIDE_POST_FAILED"); }
    public static bool Visible(IntPtr main,uint pid) { Tree(main,main,pid); return N.IsWindowVisible(main); }

    // 只读取本机 TCP owner 表，无连接/枚举网卡/访问 LAN；同时覆盖 IPv4/IPv6。
    public sealed class ListenerRow {
        public uint Owner,State;
        public int Family,Port;
        public bool Loopback;
    }
    public static bool Released(ListenerRow[] rows,uint app,uint harness,int port) {
        bool pending=false;
        foreach(var row in rows) {
            if(row.Owner==app) throw new InvalidOperationException("UNEXPECTED_OWN_LISTENER");
            if(row.Port!=port) continue;
            if(row.Owner!=harness || row.Family!=2 || !row.Loopback || row.State!=2)
                throw new InvalidOperationException("OTHER_LISTENER_DURING_BLOCKER_RELEASE");
            pending=true;
        }
        return !pending;
    }
    public static ListenerRow[] ListenerRows() {
        var rowsFound=new List<ListenerRow>();
        foreach(int family in new int[] { 2,23 }) {
            int bytes=0; uint code=GetExtendedTcpTable(IntPtr.Zero,ref bytes,false,family,3,0);
            if(code!=0 && code!=122) throw new Exception("TCP_TABLE_SIZE_FAILED");
            bool complete=false;
            for(int attempt=0;attempt<3;attempt++) {
                if(bytes<4 || bytes>8*1024*1024) throw new Exception("TCP_TABLE_SIZE_LIMIT");
                int allocated=bytes; IntPtr table=Marshal.AllocHGlobal(allocated);
                try {
                    code=GetExtendedTcpTable(table,ref bytes,false,family,3,0);
                    if(code==122) continue;
                    if(code!=0) throw new Exception("TCP_TABLE_READ_FAILED");
                    int rows=Marshal.ReadInt32(table); int stride=family==2?24:56;
                    if(rows<0 || rows>(allocated-4)/stride) throw new Exception("TCP_TABLE_ROW_LIMIT");
                    for(int i=0;i<rows;i++) {
                        IntPtr row=IntPtr.Add(table,4+i*stride);
                        uint pid=unchecked((uint)Marshal.ReadInt32(row,family==2?20:52));
                        int offset=family==2?8:20;
                        int localPort=(Marshal.ReadByte(row,offset)<<8)|Marshal.ReadByte(row,offset+1);
                        var address=new byte[family==2?4:16]; Marshal.Copy(IntPtr.Add(row,family==2?4:0),address,0,address.Length);
                        uint state=unchecked((uint)Marshal.ReadInt32(row,family==2?0:48));
                        if(state!=2 || rowsFound.Count>=131072) throw new Exception("TCP_LISTENER_STATE_OR_COUNT_LIMIT");
                        rowsFound.Add(new ListenerRow { Owner=pid,State=state,Family=family,Port=localPort,Loopback=IPAddress.IsLoopback(new IPAddress(address)) });
                    }
                    complete=true; break;
                } finally { Marshal.FreeHGlobal(table); }
            }
            if(!complete) throw new Exception("TCP_TABLE_CHANGED_REPEATEDLY");
        }
        return rowsFound.ToArray();
    }
    public static int Listeners(uint owner,int port,bool requireLoopback) {
        int total=0;
        foreach(var row in ListenerRows()) {
            if((owner!=0 && row.Owner!=owner) || (port>=0 && row.Port!=port)) continue;
            if(requireLoopback && !row.Loopback) throw new Exception("NON_LOOPBACK_LISTENER");
            total++;
        }
        return total;
    }
    static bool AllowedTarget(string target) { return target=="/__proxy/health" || target=="/smoke?probe=1"; }
    public static string Get(int port,string target,int milliseconds) {
        if(port<1 || port>65535 || !AllowedTarget(target) || milliseconds<1 || milliseconds>2000) throw new Exception("PROBE_ARGUMENTS");
        var timer=Stopwatch.StartNew();
        using(var client=new TcpClient(AddressFamily.InterNetwork)) {
            // 固定 IPAddress.Loopback，无系统代理、DNS、重定向、认证或模型 endpoint。
            if(!client.ConnectAsync(IPAddress.Loopback,port).Wait(milliseconds)) throw new Exception("PROBE_CONNECT_TIMEOUT");
            using(var stream=client.GetStream()) using(var bytes=new MemoryStream()) {
                stream.WriteTimeout=Math.Max(1,milliseconds-(int)timer.ElapsedMilliseconds);
                byte[] request=Encoding.ASCII.GetBytes("GET "+target+" HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
                stream.Write(request,0,request.Length); var buffer=new byte[4096];
                while(true) {
                    int remaining=milliseconds-(int)timer.ElapsedMilliseconds;
                    if(remaining<=0) throw new Exception("PROBE_TOTAL_TIMEOUT"); stream.ReadTimeout=remaining;
                    int n=stream.Read(buffer,0,buffer.Length); if(n==0) break;
                    if(bytes.Length+n>32768) throw new Exception("PROBE_RESPONSE_LIMIT"); bytes.Write(buffer,0,n);
                }
                string response=Encoding.ASCII.GetString(bytes.ToArray()); int split=response.IndexOf("\r\n\r\n",StringComparison.Ordinal);
                if(split<0 || !response.StartsWith("HTTP/1.1 200 ",StringComparison.Ordinal)) throw new Exception("PROBE_HTTP_NOT_OK");
                string headers=response.Substring(0,split);
                if(headers.IndexOf("Transfer-Encoding:",StringComparison.OrdinalIgnoreCase)>=0) throw new Exception("PROBE_UNEXPECTED_TRANSFER_ENCODING");
                return response.Substring(split+4);
            }
        }
    }
}
}
'@
Add-Type -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll -TypeDefinition ($titleDeclaration + "`n" + $proxyDeclaration + "`n" + $extraDeclaration)
$layoutValidation = [AutoStartUiSmokeBorrowed.Native]::Validate()
$pureValidation = [AutoStartUiSmoke.Native]::Validate()
$tokens=$null; $parseErrors=$null
$scriptAst=[Management.Automation.Language.Parser]::ParseFile($PSCommandPath,[ref]$tokens,[ref]$parseErrors)
if ($parseErrors.Count -ne 0) { throw 'PowerShell parser errors in auto-start smoke' }
$helperTypes=@{
    '[AutoStartUiSmokeBorrowed.Native]'=[AutoStartUiSmokeBorrowed.Native]
    '[AutoStartUiSmokeFixtures.ProxyUiMock]'=[AutoStartUiSmokeFixtures.ProxyUiMock]
    '[AutoStartUiSmoke.Native]'=[AutoStartUiSmoke.Native]
}
foreach ($call in $scriptAst.FindAll({ param($node) $node -is [Management.Automation.Language.InvokeMemberExpressionAst] -and $node.Static },$true)) {
    $name=$call.Expression.Extent.Text
    if (-not $helperTypes.ContainsKey($name)) { continue }
    $type=$helperTypes[$name]; $member=$call.Member.Value
    $arity=0
    if ($null -ne $call.Arguments) { $arity=@($call.Arguments).Length }
    if ($member -eq 'new') { $methods=@($type.GetConstructors() | Where-Object { @($_.GetParameters()).Length -eq $arity }) }
    else { $methods=@($type.GetMethods() | Where-Object { $_.IsStatic -and $_.Name -eq $member -and @($_.GetParameters()).Length -eq $arity }) }
    if ($methods.Count -eq 0) { throw "Missing public helper/arity: $name $member" }
}
if ($ValidateOnly) {
    Write-Output "PASS: PowerShell AST/helper call arities, C# compilation, $layoutValidation; $pureValidation"
    Write-Output 'VALIDATION ONLY: source declarations only; no EXE/config/report lookup, fixture writes, instance/Win32/network calls, process launch or GUI acceptance'
    return
}

$project=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($ExePath)) { $ExePath=Join-Path $project 'target\debug\ai-relay.exe' }
$exe=[IO.Path]::GetFullPath($ExePath)
$processNames=@('ai-relay',[IO.Path]::GetFileNameWithoutExtension($exe)) | Select-Object -Unique
$session=[Diagnostics.Process]::GetCurrentProcess().SessionId
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User
$mutexName="Local\AIRelay.$($sid.Value).$session.v1"
$workDeadline=($BudgetSeconds-24)*1000 # 预留最长一组有界消息、正常退出、mock 停止及哈希/报告的收尾时间。
$child=$null; $pin=$null; $second=$null; $window=[IntPtr]::Zero; $exitPosted=$false; $exitDeadlineMs=0
$mock=$null; $blocker=$null; $temp=$null; $output=$null; $current=$null; $tempCreated=$false
$stage='preflight'; $failure=$null; $runId=[Guid]::NewGuid().ToString('N')
$results=[Collections.Generic.List[object]]::new()
$cleanupErrors=[Collections.Generic.List[string]]::new()
$utf8=[Text.UTF8Encoding]::new($false)
$specifications=@(
    @{ Group='01-defaults'; Id='omitted-proxy'; Policy='omitted-proxy'; Kind='running'; Repeat=$true },
    @{ Group='01-defaults'; Id='omitted-auto-start'; Policy='omitted-auto-start'; Kind='running'; Repeat=$false },
    @{ Group='02-explicit-true'; Id='explicit-true'; Policy='true'; Kind='running'; Repeat=$false },
    @{ Group='03-explicit-false'; Id='explicit-false'; Policy='false'; Kind='stopped'; Repeat=$false },
    @{ Group='04-first-init'; Id='missing-config'; Policy='missing'; Kind='unconfigured'; Repeat=$false },
    @{ Group='05-validation'; Id='invalid-toml'; Policy='true'; Kind='invalid-toml'; Repeat=$false },
    @{ Group='05-validation'; Id='invalid-upstream'; Policy='true'; Kind='invalid-upstream'; Repeat=$false },
    @{ Group='05-validation'; Id='empty-upstream'; Policy='true'; Kind='unconfigured'; Repeat=$false },
    @{ Group='06-bind-failure'; Id='occupied-port'; Policy='true'; Kind='occupied'; Repeat=$true },
    @{ Group='07-lan-guard'; Id='lan-guard'; Policy='true'; Kind='lan'; Repeat=$false }
)
function Fail([string]$Code) { throw [InvalidOperationException]::new($Code) }
function Check([bool]$Condition,[string]$Code) { if (-not $Condition) { Fail $Code } }
function Safe-Code($ErrorRecord) {
    $message=$ErrorRecord.Exception.GetBaseException().Message
    if ($message -cmatch '^[A-Z][A-Z0-9_]{2,95}$') { return $message }
    return 'UNEXPECTED_FAILURE' # 不把 I/O exception 的绝对路径、GUI 全文或配置内容写入报告。
}
function Limit {
    if ($clock.ElapsedMilliseconds -ge $workDeadline) { Fail 'WORK_BUDGET_EXCEEDED' }
}
function No-Others {
    $allowed=@()
    if ($null -ne $child) { $allowed+= $child.Id }
    if ($null -ne $second) { $allowed+= $second.Id }
    $others=@(Get-Process -Name $processNames -ErrorAction SilentlyContinue | Where-Object { $_.Id -notin $allowed })
    Check ($others.Count -eq 0) 'FOREIGN_APP_PROCESS_NO_INPUT_OR_IPC'
}
function No-Instance {
    No-Others
    $existing=$null
    try { $existing=[Threading.Mutex]::OpenExisting($mutexName) }
    catch [Threading.WaitHandleCannotBeOpenedException] { }
    if ($null -ne $existing) { $existing.Dispose(); Fail 'EXISTING_SESSION_INSTANCE_NO_LAUNCH' }
}
function Identity {
    Check ($null -ne $child -and $null -ne $pin -and -not $child.HasExited) 'OWN_MAIN_NOT_LIVE'
    Check ([object]::ReferenceEquals($child,$pin.Process)) 'PROCESS_OBJECT_CHANGED'
    [AutoStartUiSmoke.Native]::Image($pin.Handle,[uint32]$pin.Id,$exe)
    Check ($child.Id -eq $pin.Id) 'PINNED_PROCESS_PID_CHANGED'
    Check ($child.Handle -eq $pin.Handle) 'PINNED_PROCESS_HANDLE_CHANGED'
    Check ($child.StartTime.ToUniversalTime().Ticks -eq $pin.StartTicks) 'PINNED_PROCESS_START_TIME_CHANGED'
    Check ($child.Path -ieq $exe) 'PINNED_PROCESS_PATH_CHANGED'
}
function Guard { Limit; No-Others; Identity }
function Status { Guard; return [AutoStartUiSmoke.Native]::Status($window,[uint32]$pin.Id) }
function Enabled([int]$Id) { Guard; return [AutoStartUiSmoke.Native]::Enabled($window,[uint32]$pin.Id,$Id) }
function Wait-State([string]$Name,[scriptblock]$Test,[int]$Milliseconds=10000) {
    $script:stage=$current.Id+'/'+$Name
    $timer=[Diagnostics.Stopwatch]::StartNew()
    while ($timer.ElapsedMilliseconds -lt $Milliseconds) {
        Guard
        if (& $Test) { return }
        Start-Sleep -Milliseconds 60
    }
    Fail 'WAIT_EXPIRED'
}
function Hash([string]$Path) {
    if (-not [IO.File]::Exists($Path)) { return $null }
    $stream=[IO.File]::OpenRead($Path); $hash=[Security.Cryptography.SHA256]::Create()
    try {
        Check ($stream.Length -le 1048576) 'CONFIG_HASH_SIZE_LIMIT'
        return ([BitConverter]::ToString($hash.ComputeHash($stream))).Replace('-','').ToLowerInvariant()
    } finally { $hash.Dispose(); $stream.Dispose() }
}
function Config-Artifacts([string]$Directory) {
    # 只遍历自有树；拒绝重解析点，不采集日志/数据库内容或完整文件名到报告。
    Check (([IO.File]::GetAttributes($Directory) -band [IO.FileAttributes]::ReparsePoint) -eq 0) 'FIXTURE_ROOT_REPARSE_POINT'
    $pending=[Collections.Generic.Stack[string]]::new(); $pending.Push($Directory); $count=0; $entries=0
    while ($pending.Count -gt 0) {
        foreach ($path in [IO.Directory]::GetFileSystemEntries($pending.Pop())) {
            $entries++; Check ($entries -le 512) 'FIXTURE_TREE_SIZE_LIMIT'
            $attributes=[IO.File]::GetAttributes($path)
            Check (($attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0) 'FIXTURE_REPARSE_POINT'
            if (($attributes -band [IO.FileAttributes]::Directory) -ne 0) { $pending.Push($path); continue }
            $name=[IO.Path]::GetFileName($path)
            if ($name -match '(?i)config|\.toml(?:$|[.~-])') { $count++ }
        }
    }
    return $count
}
function Config-Unchanged {
    $current.Result.ConfigHashAfter=Hash $current.Config
    $current.Result.ConfigArtifactsAfter=Config-Artifacts $current.Directory
    $current.Result.ConfigUnchanged=($current.Result.ConfigHashBefore -ceq $current.Result.ConfigHashAfter -and $current.Result.ConfigArtifactsBefore -eq $current.Result.ConfigArtifactsAfter)
    Check $current.Result.ConfigUnchanged 'CONFIG_CHANGED_OR_UNEXPECTED_CONFIG_WRITE'
}
function New-Reservation {
    $listener=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0)
    try {
        $listener.ExclusiveAddressUse=$true
        [AutoStartUiSmoke.Native]::NoInherit($listener)
        $listener.Start()
        [AutoStartUiSmoke.Native]::NoInherit($listener)
        return $listener
    } catch { $listener.Stop(); $listener.Server.Dispose(); throw }
}
function Fixture([hashtable]$Spec) {
    Limit; No-Instance
    $directory=Join-Path $temp ($Spec.Id+' 中文 fixture')
    Check ([IO.Path]::IsPathRooted($directory) -and -not [IO.Directory]::Exists($directory)) 'FIXTURE_PATH_NOT_NEW_ABSOLUTE'
    [void][IO.Directory]::CreateDirectory($directory)
    $config=Join-Path $directory 'config.toml'; $port=0
    if ($Spec.Policy -ne 'missing') {
        $script:blocker=New-Reservation; $port=([Net.IPEndPoint]$blocker.LocalEndpoint).Port
        $address=if ($Spec.Kind -eq 'lan') { '0.0.0.0' } else { '127.0.0.1' }
        $url=if ($Spec.Kind -eq 'invalid-upstream') { 'not-an-http-url' } elseif ($Spec.Kind -eq 'unconfigured') { '' } else { 'http://127.0.0.1:'+$mock.Port+'/v1' }
        $toml="schema_version = 1`n[upstream]`nurl = '$url'`n[listen]`naddress = '$address'`nport = $port`n"
        if ($Spec.Policy -ne 'omitted-proxy') {
            $toml+="[proxy]`nforward_enabled = false`nconnect_enabled = false`n"
            if ($Spec.Policy -ne 'omitted-auto-start') { $toml+='auto_start = '+$Spec.Policy+"`n" }
        }
        $toml+="[keys]`ninjection_mode = 'off'`n[network]`nshutdown_grace_secs = 1`n[ui]`nremember_window_geometry = false`n[system]`nlogin_auto_start = false`n"
        if ($Spec.Kind -eq 'invalid-toml') { $toml+="[unfinished`n" }
        [IO.File]::WriteAllText($config,$toml,$utf8)
    }
    $result=[pscustomobject][ordered]@{
        Group=$Spec.Group; Case=$Spec.Id; SavedAutoStart=$Spec.Policy; Expected=$Spec.Kind; Outcome='incomplete'
        OrdinaryLaunch=$false; IdentityPinned=$false; CheckboxReadOnly=$false; CheckboxChecked=$null
        HealthOnlyLocal=$false; MockRequests=0; NoOwnListener=$false; VisibleError=$false; UnconfiguredMessage=$false; LanGuardMessage=$false
        PortReleaseWaitMs=0; PortReleaseObservationMs=0; PortReleaseDiagnostics=@(); DuplicateOrdinaryLaunchOnlyShow=$false; StopRouteUsed=$false; LanLoopbackInterlock=($Spec.Kind -eq 'lan')
        ConfigHashBefore=(Hash $config); ConfigHashAfter=$null; ConfigArtifactsBefore=(Config-Artifacts $directory); ConfigArtifactsAfter=$null; ConfigUnchanged=$false
        NormalExit=$false; Seconds=0
    }
    $results.Add($result)
    $script:current=[pscustomobject]@{ Id=$Spec.Id; Directory=$directory; Config=$config; Port=$port; Result=$result; RequestsBefore=$mock.Requests; StartedMs=$clock.ElapsedMilliseconds }
    # 非 occupied/LAN 用例在普通启动前释放端口；出现竞争即失败，不换端口/重试启动。
    if ($null -ne $blocker -and $Spec.Kind -notin @('occupied','lan')) { $blocker.Stop(); $script:blocker=$null }
}
function Ordinary-Launch([string]$Directory) {
    Limit
    Check ([IO.Path]::IsPathRooted($Directory) -and -not $Directory.Contains('"')) 'LAUNCH_DIRECTORY_NOT_ABSOLUTE_OR_QUOTABLE'
    $info=[Diagnostics.ProcessStartInfo]::new()
    $info.FileName=$exe; $info.WorkingDirectory=$Directory; $info.UseShellExecute=$false
    $info.Arguments='--config-dir "'+$Directory+'"' # 唯一参数；没有 start/show/stop/status/--background。
    return [Diagnostics.Process]::Start($info)
}
function Launch-Main {
    No-Instance
    $script:stage=$current.Id+'/ordinary-launch'
    $script:child=Ordinary-Launch $current.Directory
    $script:pin=[pscustomobject]@{ Process=$child; Id=$child.Id; Handle=$child.Handle; StartTicks=$child.StartTime.ToUniversalTime().Ticks }
    $script:window=[IntPtr]::Zero; $script:exitPosted=$false; $script:exitDeadlineMs=0
    # 刚创建进程的托管 Path 元信息可能暂时为空；保留原始 handle 并有界刷新，
    # 每次先用该 handle 查询真实 EXE；非空不匹配直接拒绝，绝不重开 PID/放宽身份。
    $identityTimer=[Diagnostics.Stopwatch]::StartNew()
    do {
        Limit; No-Others
        [AutoStartUiSmoke.Native]::Image($pin.Handle,[uint32]$pin.Id,$exe)
        $child.Refresh()
        Check ($child.Id -eq $pin.Id -and $child.Handle -eq $pin.Handle -and $child.StartTime.ToUniversalTime().Ticks -eq $pin.StartTicks) 'PINNED_PROCESS_CORE_IDENTITY_CHANGED'
        $path=$child.Path
        if (-not [string]::IsNullOrWhiteSpace($path)) {
            Check ($path -ieq $exe) 'PINNED_PROCESS_PATH_CHANGED'
            break
        }
        Check ($identityTimer.ElapsedMilliseconds -lt 5000) 'PROCESS_PATH_METADATA_TIMEOUT'
        Start-Sleep -Milliseconds 50
    } while ($true)
    Identity; $current.Result.IdentityPinned=$true; $current.Result.OrdinaryLaunch=$true
    Wait-State 'own-main-window' {
        $script:window=[AutoStartUiSmokeBorrowed.Native]::FindMain([uint32]$pin.Id)
        $window -ne [IntPtr]::Zero -and [AutoStartUiSmoke.Native]::Visible($window,[uint32]$pin.Id)
    }
}
function Port-Diagnostic([string]$Phase,[object[]]$Rows) {
    $matches=@($Rows | Where-Object { $_.Port -eq $current.Port })
    $current.Result.PortReleaseDiagnostics+=@([pscustomobject]@{
        Phase=$Phase; ElapsedMs=$clock.ElapsedMilliseconds-$current.StartedMs
        Rows=@($matches | ForEach-Object {
            [pscustomobject]@{ Family=$_.Family; State=$_.State; Loopback=$_.Loopback; Owner=$(if ($_.Owner -eq $pin.Id) { 'APP' } elseif ($_.Owner -eq $PID) { 'HARNESS' } else { 'OTHER' }) }
        })
    })
}
function No-Listener([switch]$PortFree) {
    Guard
    # APP 与全局同号端口使用同批枚举行；报告只记owner角色/地址类别，不记PID或实际IP。
    $rows=@([AutoStartUiSmoke.Native]::ListenerRows())
    Check (@($rows | Where-Object { $_.Owner -eq $pin.Id }).Count -eq 0) 'UNEXPECTED_OWN_LISTENER'
    if ($PortFree -and $current.Port -gt 0 -and @($rows | Where-Object { $_.Port -eq $current.Port }).Count -ne 0) {
        Port-Diagnostic 'unexpected-port-listener' $rows
        Fail 'TEST_PORT_NOT_FREE'
    }
    $current.Result.NoOwnListener=$true
}
function Stopped([string]$Kind) {
    $value=Status
    if ($value -notmatch '^代理状态：(已停止|未配置|错误)' -or (Enabled 2103)) { return $false }
    switch ($Kind) {
        'invalid-toml' { return $value -match '配置格式|版本不受支持|配置.*(?:失败|错误|损坏)' }
        'invalid-upstream' { return $value -match 'upstream\.url|上游.*(?:无效|非法|校验|错误)' }
        'occupied' { return $value -match '无法监听|端口.*占用|监听.*(?:失败|错误)' }
        'unconfigured' { return $value -match '首次|初始化|未配置|未保存|尚未.*(?:配置|保存|填写|上游)|上游.*(?:为空|未填写|未设置)|请.*(?:配置|保存|填写|设置)' }
        'lan' { return $value -match '非回环|局域网' -and $value -match '确认|手动|点击' -and $value -notmatch '无法监听|端口.*占用' }
        default { return (Enabled 2102) }
    }
}
function Observe-Stopped([string]$Kind,[int]$Milliseconds=1200,[switch]$PortFree) {
    $timer=[Diagnostics.Stopwatch]::StartNew()
    do {
        Guard
        Check ([AutoStartUiSmoke.Native]::Visible($window,[uint32]$pin.Id)) 'STOPPED_GUI_NOT_VISIBLE'
        Check (Stopped $Kind) 'STOPPED_STATE_OR_VISIBLE_MESSAGE_CHANGED'
        No-Listener -PortFree:$PortFree
        Start-Sleep -Milliseconds 80
    } while ($timer.ElapsedMilliseconds -lt $Milliseconds)
}
function Checkbox([bool]$Expected) {
    Guard; [AutoStartUiSmoke.Native]::Page($window,[uint32]$pin.Id,3)
    Wait-State 'settings-tab' { [AutoStartUiSmoke.Native]::PageIs($window,[uint32]$pin.Id,3) }
    Guard; [AutoStartUiSmoke.Native]::StartupCategory($window,[uint32]$pin.Id)
    Wait-State 'category-4-auto-start-checkbox' {
        [AutoStartUiSmoke.Native]::CategoryIs($window,[uint32]$pin.Id) -and [AutoStartUiSmoke.Native]::CheckboxVisible($window,[uint32]$pin.Id)
    }
    Guard; $checked=[AutoStartUiSmoke.Native]::Checkbox($window,[uint32]$pin.Id)
    $current.Result.CheckboxChecked=($checked -eq 1); $current.Result.CheckboxReadOnly=$true
    Check (($checked -eq 1) -eq $Expected) 'CHECKBOX_DID_NOT_PRESERVE_AUTO_START'
    Config-Unchanged
    Guard; [AutoStartUiSmoke.Native]::Page($window,[uint32]$pin.Id,0)
    Wait-State 'overview-tab' { [AutoStartUiSmoke.Native]::PageIs($window,[uint32]$pin.Id,0) }
}
function Running {
    Wait-State 'automatic-running-without-start-command' { (Status) -match '^代理状态：运行中' -and (Enabled 2103) -and -not (Enabled 2102) }
    Guard
    Check ([AutoStartUiSmoke.Native]::Listeners([uint32]$pin.Id,-1,$true) -eq 1 -and [AutoStartUiSmoke.Native]::Listeners([uint32]$pin.Id,$current.Port,$true) -eq 1) 'EXPECTED_OWN_LOOPBACK_LISTENER_MISSING'
    $body=[AutoStartUiSmoke.Native]::Get($current.Port,'/__proxy/health',2000)
    $health=$body | ConvertFrom-Json
    Check (@($health.PSObject.Properties).Count -eq 2 -and $health.ok -eq $true -and $health.uptime_secs -ge 0) 'HEALTH_SHAPE_OR_PRIVACY'
    Check ($mock.Requests -eq $current.RequestsBefore) 'HEALTH_REACHED_UPSTREAM'
    $current.Result.HealthOnlyLocal=$true
    Guard
    Check ([AutoStartUiSmoke.Native]::Get($current.Port,'/smoke?probe=1',2000) -ceq $marker) 'LOCAL_MOCK_RESPONSE_MISMATCH'
    Wait-State 'exactly-one-local-mock-request' { $mock.Requests -eq $current.RequestsBefore+1 }
    Check ($mock.ProxyCredentialsReceived -eq 0) 'LOCAL_MOCK_AUTH_LEAK'
    $current.Result.MockRequests=1
    Checkbox $true
}
function Duplicate-OnlyShow([string]$Kind) {
    if ($Kind -eq 'stopped') {
        Guard; Check (Enabled 2103) 'OWN_STOP_NOT_ENABLED'
        [AutoStartUiSmoke.Native]::Command($window,[uint32]$pin.Id,2103)
        $current.Result.StopRouteUsed=$true
        Wait-State 'own-stop-completed' { (Stopped 'stopped') -and [AutoStartUiSmoke.Native]::Listeners([uint32]$pin.Id,-1,$false) -eq 0 }
        Observe-Stopped 'stopped' -PortFree
    }
    Guard; [AutoStartUiSmoke.Native]::Hide($window,[uint32]$pin.Id)
    Wait-State 'own-main-hidden' { -not [AutoStartUiSmoke.Native]::Visible($window,[uint32]$pin.Id) }
    $script:stage=$current.Id+'/duplicate-ordinary-launch'
    $script:second=Ordinary-Launch $current.Directory
    # 保留原始 Process 的 handle 至退出确认；不根据 PID 重开/发送命令给第二进程。
    $secondHandle=$second.Handle
    $timer=[Diagnostics.Stopwatch]::StartNew()
    while (-not $second.WaitForExit(40)) {
        Guard; No-Listener -PortFree
        if ($timer.ElapsedMilliseconds -ge 7000) { Fail 'SECOND_ORDINARY_LAUNCH_DID_NOT_EXIT' }
    }
    Check ($second.Handle -eq $secondHandle -and $second.ExitCode -eq 0) 'SECOND_ORDINARY_LAUNCH_FAILED'
    Wait-State 'original-main-shown-by-second-launch' { [AutoStartUiSmoke.Native]::Visible($window,[uint32]$pin.Id) }
    Identity
    Observe-Stopped $Kind 2500 -PortFree
    $current.Result.DuplicateOrdinaryLaunchOnlyShow=$true
    $second.Dispose(); $script:second=$null
    Config-Unchanged
}
function Exit-Owned {
    if ($null -eq $child) { return }
    $script:stage=$current.Id+'/normal-exit-1002'
    if ($exitDeadlineMs -eq 0) { $script:exitDeadlineMs=[Math]::Min($clock.ElapsedMilliseconds+10000,$BudgetSeconds*1000-5000) }
    if (-not $child.HasExited -and -not $exitPosted) {
        # 收尾不依赖 workDeadline，但仍遵守总预算/原始 handle/无其他实例及自有 HWND 树。
        No-Others; Identity
        if ($window -eq [IntPtr]::Zero) { $script:window=[AutoStartUiSmokeBorrowed.Native]::FindMain([uint32]$pin.Id) }
        [AutoStartUiSmoke.Native]::Command($window,[uint32]$pin.Id,1002)
        $script:exitPosted=$true
    }
    while (-not $child.WaitForExit(40)) {
        No-Others
        if ($clock.ElapsedMilliseconds -ge $exitDeadlineMs) { Fail 'NORMAL_EXIT_TIMEOUT_NO_KILL_FALLBACK' }
    }
    Check ($exitPosted -and $child.ExitCode -eq 0) 'OWN_MAIN_EXITED_UNEXPECTEDLY'
    $current.Result.NormalExit=$true
}
function Finish-Fixture {
    Config-Unchanged
    Check ($mock.Requests -eq $current.RequestsBefore+$current.Result.MockRequests) 'UNEXPECTED_UPSTREAM_REQUEST'
    Exit-Owned
    Config-Unchanged # 正常退出后再验哈希，包含 flush，不只比较启动后的瞬间。
    $current.Result.Seconds=[Math]::Round(($clock.ElapsedMilliseconds-$current.StartedMs)/1000,2)
    $current.Result.Outcome='passed'
    Write-Host ('PASS fixture: '+$current.Id+'; config unchanged; normal exit')
    $child.Dispose(); $script:child=$null; $script:pin=$null; $script:window=[IntPtr]::Zero
    if ($null -ne $blocker) { $blocker.Stop(); $script:blocker=$null }
}

try {
    # 先拒绝已有实例，再查询 EXE/创建报告及 fixture；绝不把测试参数发给现有用户实例。
    Limit; No-Instance
    Check ([IO.Path]::IsPathRooted($exe) -and [IO.File]::Exists($exe)) 'EXE_NOT_FOUND_BUILD_LATEST_OUTSIDE_THIS_SCRIPT'
    $exe=(Get-Item -LiteralPath $exe).FullName
    $output=Join-Path (Join-Path $project 'target\auto-start-ui') ([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')+'-'+$runId)
    [void][IO.Directory]::CreateDirectory($output)
    $temp=[IO.Path]::GetFullPath((Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-auto-start-ui-'+$runId)))
    Check (-not [IO.Directory]::Exists($temp) -and -not [IO.File]::Exists($temp)) 'TEMP_ROOT_ALREADY_EXISTS'
    $acl=[Security.AccessControl.DirectorySecurity]::new()
    $acl.SetAccessRuleProtection($true,$false)
    $rule=[Security.AccessControl.FileSystemAccessRule]::new($sid,[Security.AccessControl.FileSystemRights]::FullControl,([Security.AccessControl.InheritanceFlags]::ContainerInherit -bor [Security.AccessControl.InheritanceFlags]::ObjectInherit),[Security.AccessControl.PropagationFlags]::None,[Security.AccessControl.AccessControlType]::Allow)
    $acl.AddAccessRule($rule)
    [void][IO.Directory]::CreateDirectory($temp,$acl); $tempCreated=$true
    $marker='auto-start-local-mock-'+$runId
    $mock=[AutoStartUiSmokeFixtures.ProxyUiMock]::new($marker)
    foreach ($spec in $specifications) {
        $stage=$spec.Id+'/fixture'
        Fixture $spec
        Launch-Main
        switch ($spec.Kind) {
            'running' { Running; if ($spec.Repeat) { Duplicate-OnlyShow 'stopped' } }
            'stopped' {
                Wait-State 'explicit-false-stays-stopped' { Stopped 'stopped' }
                Checkbox $false; Observe-Stopped 'stopped' -PortFree
            }
            'occupied' {
                Wait-State 'automatic-bind-failure-visible' { Stopped 'occupied' }
                $current.Result.VisibleError=$true
                Observe-Stopped 'occupied'
                Checkbox $true
                Guard
                Check ($blocker.LocalEndpoint.Port -eq $current.Port -and [object]::ReferenceEquals($blocker,$script:blocker)) 'BLOCKER_IDENTITY_CHANGED'
                $before=@([AutoStartUiSmoke.Native]::ListenerRows())
                $occupiedRows=@($before | Where-Object { $_.Port -eq $current.Port })
                Check ($occupiedRows.Count -eq 1 -and $occupiedRows[0].Owner -eq $PID -and $occupiedRows[0].Family -eq 2 -and $occupiedRows[0].Loopback) 'BLOCKER_LISTENER_NOT_EXCLUSIVE'
                Port-Diagnostic 'before-blocker-stop' $before
                $blocker.Stop(); $blocker=$null
                $after=@([AutoStartUiSmoke.Native]::ListenerRows())
                Port-Diagnostic 'after-blocker-stop' $after
                [void][AutoStartUiSmoke.Native]::Released($after,[uint32]$pin.Id,[uint32]$PID,$current.Port)
                # Windows owner表可短暂保留已关闭的HARNESS项；只允许原占位角色/族，
                # 期间APP任何监听都失败。不重绑端口，确认零监听后才开始完整无重试观察。
                $releaseTimer=[Diagnostics.Stopwatch]::StartNew()
                Wait-State 'blocker-listener-fully-released' {
                    $rows=@([AutoStartUiSmoke.Native]::ListenerRows())
                    $current.Result.PortReleaseWaitMs=$releaseTimer.ElapsedMilliseconds
                    if ([AutoStartUiSmoke.Native]::Released($rows,[uint32]$pin.Id,[uint32]$PID,$current.Port)) {
                        Port-Diagnostic 'blocker-fully-released' $rows
                        return $true
                    }
                    return $false
                } 4000
                # 释放占用后持续观察；不能以再次显式启动/换端口掩盖错误或自动重试。
                $stage=$current.Id+'/port-release-no-retry-observation'
                Observe-Stopped 'occupied' 2500 -PortFree
                $current.Result.PortReleaseObservationMs=2500
                Duplicate-OnlyShow 'occupied'
            }
            'lan' {
                # 独占回环占位贯穿此用例，阻止错误实现意外绑定 0.0.0.0；不连接任何 LAN 地址。
                # 必须看到 LAN 手动确认说明，单纯 bind failure 不算 guard 通过。
                Wait-State 'lan-guard-visible-without-authorization' { Stopped 'lan' }
                $current.Result.LanGuardMessage=$true
                Checkbox $true; Observe-Stopped 'lan'
            }
            'unconfigured' {
                Wait-State 'first-init-or-empty-upstream-stopped' { Stopped 'unconfigured' }
                $current.Result.UnconfiguredMessage=$true
                Checkbox $true; Observe-Stopped 'unconfigured' -PortFree
            }
            default {
                Wait-State 'invalid-config-visible-stopped-error' { Stopped $spec.Kind }
                $current.Result.VisibleError=$true
                Observe-Stopped $spec.Kind -PortFree
            }
        }
        Finish-Fixture
    }
} catch {
    $failure=[pscustomobject]@{ Code=(Safe-Code $_); Stage=$stage; ExceptionType=$_.Exception.GetBaseException().GetType().Name }
    if ($null -ne $current -and $current.Result.Outcome -ne 'passed') { $current.Result.Outcome='failed' }
    Write-Warning ('FAIL: '+$failure.Stage+' / '+$failure.Code+' (no raw exception/config/path recorded)')
} finally {
    # 不调用 CLI quit/Stop-Process/Kill，不操作第三方 HWND。失败也只能走一次自有 1002。
    if ($null -ne $child) {
        try { Exit-Owned } catch { $cleanupErrors.Add((Safe-Code $_)) }
        if ($null -ne $current) {
            try { Config-Unchanged } catch { $cleanupErrors.Add((Safe-Code $_)) }
        }
    }
    $mainLive=($null -ne $child -and -not $child.HasExited)
    $secondLive=($null -ne $second -and -not $second.HasExited)
    if ($secondLive) { $cleanupErrors.Add('SECOND_PROCESS_RETAINED_NO_KILL') }
    if ($mainLive) { $cleanupErrors.Add('OWN_MAIN_RETAINED_NO_KILL') }
    if ($null -ne $blocker) { $blocker.Stop() }
    if ($null -ne $mock) { try { $mock.Dispose() } catch { $cleanupErrors.Add('MOCK_SHUTDOWN_FAILED') } }
    $tempRetained=$tempCreated
    if ($tempCreated -and -not $mainLive -and -not $secondLive) {
        try {
            [void](Config-Artifacts $temp) # 先拒绝树中 reparse point，再删除唯一自有根目录。
            Remove-Item -LiteralPath $temp -Recurse -Force
            $tempRetained=$false
        } catch { $cleanupErrors.Add('PRIVATE_FIXTURE_CLEANUP_FAILED') }
    }
    if ($clock.ElapsedMilliseconds -ge $BudgetSeconds*1000) { $cleanupErrors.Add('TOTAL_BUDGET_EXCEEDED') }
    if ($null -ne $output) {
        $report=[ordered]@{
            Complete=$false # Win32 定向冒烟不冒称物理 UI/Windows 全验收。
            AutomatedScopeComplete=($null -eq $failure -and $cleanupErrors.Count -eq 0 -and $results.Count -eq $specifications.Count -and @($results | Where-Object { $_.Outcome -ne 'passed' }).Count -eq 0)
            RunId=$runId; BudgetSeconds=$BudgetSeconds; Seconds=[Math]::Round($clock.Elapsed.TotalSeconds,2)
            GroupCount=7; FixtureCount=$specifications.Count; HarnessSocketsNonInheritable=$true; Failure=$failure; Cases=$results.ToArray(); CleanupErrors=$cleanupErrors.ToArray()
            TempRetained=$tempRetained; RetainedOwnedPid=$(if ($mainLive) { $child.Id } else { $null })
            Limits=@(
                'Requires the latest EXE with checkbox ID 2065 and caption contract; no build or controller/unit tests are run here.',
                'Only ordinary Launch --config-dir absolute is used, including duplicate launches. No CLI start/restart or GUI Start dispatch. Only owned main Stop 2103 and Exit 1002 are dispatched.',
                'The checkbox is inspected by class/style/caption/BM_GETCHECK in Settings category 4; never clicked or saved. Startup controls, firewall, system proxy and clipboard are never changed. Newly owned mock/reservation socket handles have inheritance cleared and verified before child launch; no foreign or child socket handle is changed.',
                'Only local health and one fixed local-mock GET per running fixture; no model-list/API requests, real keys, DNS, redirects, external targets or costs.',
                'LAN uses 0.0.0.0 only as fixture data with an exclusive loopback port interlock. Both TCP owner tables must show zero owned listeners and GUI must explain manual LAN authorization, not bind failure. No LAN connection is made; bind attempts are not instrumented.',
                'No-retry and duplicate-show checks are bounded 2.5s observations after releasing the occupied port and after duplicate exit; they do not prove absence of arbitrarily delayed retries. Before the no-retry window, at most 4s is allowed only for the original HARNESS IPv4-loopback listener row to disappear; APP listeners are rejected throughout and the port is never rebound.',
                'Status/error evidence is visible native HWND text and enabled state, not screenshots/physical input/DPI/high-contrast/reader acceptance.',
                'Main Process object, PID, start time, original handle and executable are pinned before owned-tree UI actions. Existing-session mutex/process preflight is fail-closed, not protection against a hostile concurrent same-user process.',
                'Config hashes/artifact counts are compared before, during and after normal exit, including false and missing config; snapshots are not an audit of transient filesystem writes.',
                'Normal exit has no kill fallback. A hung owned process/fixture is retained and reported; the shared mock and loopback interlock end with this script. No raw GUI/config/exception text or absolute machine paths are included in this report.'
            )
        }
        try { [IO.File]::WriteAllText((Join-Path $output 'report.json'),($report | ConvertTo-Json -Depth 10),$utf8) }
        catch { $cleanupErrors.Add('REPORT_WRITE_FAILED') }
        Write-Host ('Report: target/auto-start-ui/'+[IO.Path]::GetFileName($output)+'/report.json')
    }
    if ($null -ne $second) { $second.Dispose() }
    if ($null -ne $child) { $child.Dispose() }
}
if ($null -ne $failure) { throw ('AUTO_START_UI_FAILED: '+$failure.Code+'; stage='+$failure.Stage) }
if ($cleanupErrors.Count -ne 0) { throw ('AUTO_START_UI_CLEANUP_FAILED: '+($cleanupErrors -join '; ')) }
Write-Output 'PASS: all 7 automated groups / 10 private fixtures; inspect target/auto-start-ui report Limits for remaining acceptance'
