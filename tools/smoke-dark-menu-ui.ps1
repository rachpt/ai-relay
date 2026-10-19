# Windows PowerShell 5.1 x64. Private stopped fixture; no network/keys/clipboard/Startup.
[CmdletBinding()]
param([string]$ExePath, [switch]$ValidateOnly)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$source = @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading.Tasks;
public static class DarkMenuSmoke {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int L,T,R,B; }
    [StructLayout(LayoutKind.Sequential)] struct Mii {
        public uint Size,Mask,Type,State,Id;
        public IntPtr Sub,Checked,Unchecked,Data,Text;
        public uint Length; public IntPtr Bitmap;
    }
    [StructLayout(LayoutKind.Sequential)] struct GuiInfo { public uint Size,Flags; public IntPtr Active,Focus,Capture,MenuOwner,MoveSize,Caret; public Rect CaretRect; }
    public sealed class Item { public uint Id,Type,State; public string Text; public IntPtr Sub; }
    public sealed class Image { public int Width,Height,Dark,Light,Pixels,FirstRgb; public bool SolidRgb; public string Path; }
    delegate bool EnumProc(IntPtr h,IntPtr unused);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc proc,IntPtr unused);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h,EnumProc proc,IntPtr unused);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h,StringBuilder s,int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
    [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr GetSystemMenu(IntPtr h,bool revert);
    [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr menu);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern bool GetMenuItemInfoW(IntPtr m,uint i,bool pos,ref Mii info);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h,out Rect r);
    [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint thread,ref GuiInfo info);
    [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h,IntPtr dc,uint flags);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern uint GetGuiResources(IntPtr process,uint flags);
    [DllImport("user32.dll")] static extern uint MapVirtualKeyW(uint key,uint mode);
    [DllImport("user32.dll",SetLastError=true)] static extern bool PostMessageW(IntPtr h,uint m,UIntPtr w,IntPtr l);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr h,uint m,UIntPtr w,IntPtr l,uint flags,uint timeout,out UIntPtr result);
    [DllImport("user32.dll",CharSet=CharSet.Unicode,EntryPoint="SendMessageTimeoutW")] static extern IntPtr ReadText(IntPtr h,uint m,UIntPtr w,StringBuilder l,uint flags,uint timeout,out UIntPtr result);
    [DllImport("kernel32.dll")] static extern uint GetProcessId(IntPtr process);
    [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr process,uint timeout);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode)] static extern bool QueryFullProcessImageNameW(IntPtr process,uint flags,StringBuilder path,ref uint capacity);
    public static void Validate() { if(IntPtr.Size!=8 || Marshal.SizeOf(typeof(Mii))!=80) throw new Exception("x64 ABI required"); }
    public static uint Owner(IntPtr h) { uint pid; GetWindowThreadProcessId(h,out pid); return pid; }
    public static string Class(IntPtr h) { var s=new StringBuilder(256); GetClassNameW(h,s,s.Capacity); return s.ToString(); }
    public static void Check(IntPtr h,uint pid) { if(h==IntPtr.Zero || Owner(h)!=pid) throw new Exception("Foreign/missing HWND refused"); }
    public static void ProcessCheck(IntPtr handle,uint pid,string exe) {
        if(GetProcessId(handle)!=pid || WaitForSingleObject(handle,0)!=258) throw new Exception("Pinned process exited/changed");
        var path=new StringBuilder(32768);uint capacity=(uint)path.Capacity;
        if(!QueryFullProcessImageNameW(handle,0,path,ref capacity) || !String.Equals(path.ToString(),exe,StringComparison.OrdinalIgnoreCase)) throw new Exception("Pinned executable mismatch/unavailable");
    }
    public static IntPtr FindMain(uint pid) {
        IntPtr found=IntPtr.Zero;
        EnumWindows(delegate(IntPtr h,IntPtr unused) { if(Owner(h)==pid && Class(h)=="AIRelay.Window.v1") found=h; return true; },IntPtr.Zero);
        return found;
    }
    public static IntPtr[] Menus(uint pid) {
        var list=new List<IntPtr>();
        EnumWindows(delegate(IntPtr h,IntPtr unused) { if(Owner(h)==pid && IsWindowVisible(h) && Class(h)=="#32768") list.Add(h); return true; },IntPtr.Zero);
        return list.ToArray();
    }
    public static IntPtr Control(IntPtr parent,uint pid,int id) {
        Check(parent,pid); IntPtr found=IntPtr.Zero;
        EnumChildWindows(parent,delegate(IntPtr h,IntPtr unused) { if(Owner(h)==pid && GetDlgCtrlID(h)==id) { if(found!=IntPtr.Zero) throw new Exception("Duplicate ID"); found=h; } return true; },IntPtr.Zero);
        if(found!=IntPtr.Zero) Check(found,pid); return found;
    }
    public static long Message(IntPtr h,uint pid,uint message,ulong w,long l) {
        Check(h,pid); UIntPtr result;
        if(SendMessageTimeoutW(h,message,new UIntPtr(w),new IntPtr(l),0x22,1000,out result)==IntPtr.Zero) throw new Exception("Message timed out");
        return unchecked((long)result.ToUInt64());
    }
    public static string Text(IntPtr h,uint pid) {
        Check(h,pid); var buffer=new StringBuilder(4096); UIntPtr result;
        if(ReadText(h,13,new UIntPtr((uint)buffer.Capacity),buffer,0x22,1000,out result)==IntPtr.Zero) throw new Exception("Read text timeout");
        return buffer.ToString();
    }
    public static void Post(IntPtr h,uint pid,uint message,ulong w,long l) {
        Check(h,pid); if(!PostMessageW(h,message,new UIntPtr(w),new IntPtr(l))) throw new Exception("Post failed");
    }
    public static void Key(IntPtr h,uint pid,uint key,bool alt) {
        uint bits=1u|(MapVirtualKeyW(key,0)<<16); if(key>=0x21 && key<=0x28) bits|=0x01000000;
        if(alt) bits|=0x20000000;
        Post(h,pid,alt?0x104u:0x100u,key,unchecked((int)bits));
        Post(h,pid,alt?0x105u:0x101u,key,unchecked((int)(bits|0xC0000000)));
    }
    public static bool Foreground(IntPtr h,uint pid) {
        Check(h,pid); SetForegroundWindow(h); return Owner(GetForegroundWindow())==pid;
    }
    public static bool InMenu(IntPtr h,uint pid) { Check(h,pid); uint owner; uint thread=GetWindowThreadProcessId(h,out owner); var info=new GuiInfo { Size=(uint)Marshal.SizeOf(typeof(GuiInfo)) }; if(!GetGUIThreadInfo(thread,ref info)) throw new Exception("No GUI thread info"); return (info.Flags&0x1C)!=0; }
    public static IntPtr Root(IntPtr h,uint pid) { Check(h,pid); var m=GetSystemMenu(h,false); if(m==IntPtr.Zero) throw new Exception("No system menu"); return m; }
    public static Item[] Items(IntPtr menu) {
        int count=GetMenuItemCount(menu); if(count<0 || count>64) throw new Exception("Bad item count");
        var list=new List<Item>();
        for(uint i=0;i<count;i++) {
            var info=new Mii { Size=80,Mask=0x107 };
            if(!GetMenuItemInfoW(menu,i,true,ref info)) throw new Exception("Menu info failed");
            IntPtr buffer=Marshal.AllocHGlobal(1024);
            try {
                var text=new Mii { Size=80,Mask=0x40,Text=buffer,Length=512 };
                if(!GetMenuItemInfoW(menu,i,true,ref text) || text.Length>=511) throw new Exception("Menu text failed/truncated");
                list.Add(new Item { Id=info.Id,Type=info.Type,State=info.State,Sub=info.Sub,Text=Marshal.PtrToStringUni(buffer,(int)text.Length) });
            } finally { Marshal.FreeHGlobal(buffer); }
        }
        return list.ToArray();
    }
    public static uint Resources(IntPtr process) { return GetGuiResources(process,0); }
    public static IntPtr Dpi(IntPtr context) { return SetThreadDpiAwarenessContext(context); }
    public static Image Capture(IntPtr h,uint pid,string path) {
        Check(h,pid); Rect r; if(!GetWindowRect(h,out r)) throw new Exception("No rect");
        int width=r.R-r.L,height=r.B-r.T;
        if(width<8 || height<8 || width>4096 || height>4096) throw new Exception("Capture bounds invalid");
        var task=Task.Run(delegate {
            using(var b=new Bitmap(width,height,PixelFormat.Format32bppArgb)) {
                using(var g=Graphics.FromImage(b)) { g.Clear(Color.Magenta); var dc=g.GetHdc(); bool ok; try { ok=PrintWindow(h,dc,2); } finally { g.ReleaseHdc(dc); } if(!ok) throw new Exception("PrintWindow failed"); }
                var result=new Image { Width=width,Height=height,Path=path,SolidRgb=true,FirstRgb=b.GetPixel(3,3).ToArgb()&0xFFFFFF };
                for(int y=3;y<height-3;y++) for(int x=3;x<width-3;x++) { var c=b.GetPixel(x,y); result.Pixels++; if((c.ToArgb()&0xFFFFFF)!=result.FirstRgb) result.SolidRgb=false; if(c.R<110 && c.G<110 && c.B<110) result.Dark++; if(c.R>210 && c.G>210 && c.B>210) result.Light++; }
                b.Save(path,ImageFormat.Png); return result;
            }
        });
        if(!task.Wait(2500)) throw new Exception("PrintWindow exceeded 2.5s"); return task.Result;
    }
}
'@
Add-Type -TypeDefinition $source -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll
[DarkMenuSmoke]::Validate()
$tokens=$null; $errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($PSCommandPath,[ref]$tokens,[ref]$errors)
if ($errors.Count -ne 0) { throw 'Script parse failure' }
$calls=$ast.FindAll({param($n) $n -is [Management.Automation.Language.InvokeMemberExpressionAst] -and $n.Static -and $n.Expression.Extent.Text -eq '[DarkMenuSmoke]'},$true)
foreach ($call in $calls) {
    $arity=if ($null -eq $call.Arguments) { 0 } else { $call.Arguments.Count }
    if (@([DarkMenuSmoke].GetMethods() | Where-Object { $_.Name -eq $call.Member.Value -and @($_.GetParameters()).Count -eq $arity }).Count -eq 0) { throw "Unknown helper/arity: $($call.Member.Value)" }
}
if ($ValidateOnly) { 'PASS: parser/helper arity, C# compilation, x64 ABI; no GUI/process/files/network'; return }
if ([string]::IsNullOrWhiteSpace($ExePath)) { $ExePath=Join-Path $PSScriptRoot '..\target\debug\ai-relay.exe' }
$exe=(Get-Item -LiteralPath $ExePath).FullName
$session=[Diagnostics.Process]::GetCurrentProcess().SessionId
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$mutexName="Local\AIRelay.$sid.$session.v1"
$child=$null; $handle=[IntPtr]::Zero; $ownedPid=0; $window=[IntPtr]::Zero
$temp=$null; $output=$null; $failure=$null; $normalExit=$false; $configUnchanged=$false
$shots=[Collections.Generic.List[object]]::new(); $resources=[Collections.Generic.List[object]]::new()
$clock=[Diagnostics.Stopwatch]::StartNew(); $stage='preflight'
$oldDpi=[IntPtr]::Zero; $dpi=0; $baseline=0
function No-Foreign {
    $names=@('ai-relay',[IO.Path]::GetFileNameWithoutExtension($exe)) | Select-Object -Unique
    $others=@(Get-Process -Name $names -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $ownedPid })
    if ($others.Count -ne 0) { throw 'Existing/foreign app found; refused' }
}
function Guard([switch]$Cleanup) {
    No-Foreign
    if (-not $Cleanup -and $clock.Elapsed.TotalSeconds -gt 150) { throw "150s budget exceeded: $stage" }
    if ($null -eq $child -or $child.HasExited -or $child.Id -ne $ownedPid -or $child.Handle -ne $handle) { throw 'Pinned child missing' }
    [DarkMenuSmoke]::ProcessCheck($handle,[uint32]$ownedPid,$exe)
}
function Wait-For([scriptblock]$Test,[int]$Timeout=5000) {
    $timer=[Diagnostics.Stopwatch]::StartNew()
    while ($timer.ElapsedMilliseconds -lt $Timeout) { Guard; if (& $Test) { return }; Start-Sleep -Milliseconds 25 }
    throw "Wait expired: $stage"
}
function C([int]$Id) { Guard; [DarkMenuSmoke]::Control($window,[uint32]$ownedPid,$Id) }
function Key([uint32]$Code,[switch]$Alt) {
    Guard; $target=$window; $popups=@([DarkMenuSmoke]::Menus([uint32]$ownedPid))
    if ($popups.Count -gt 0) { $target=$popups[0] }
    [DarkMenuSmoke]::Key($target,[uint32]$ownedPid,$Code,[bool]$Alt)
}
function Open([string]$Method='F10') {
    Guard; [DarkMenuSmoke]::Foreground($window,[uint32]$ownedPid) | Out-Null
    # Targeted HWND messages and OFFSCREEN capture do not require foreground.
    # Do not steal focus or treat this as physical keyboard/visibility evidence.
    switch ($Method) { 'F10' { Key 0x79 }; 'AltM' { Key 0x4D -Alt }; 'AltSpace' { Key 0x20 -Alt } }
    Wait-For { [DarkMenuSmoke]::InMenu($window,[uint32]$ownedPid) -and @([DarkMenuSmoke]::Menus([uint32]$ownedPid)).Count -eq 1 }
    return @([DarkMenuSmoke]::Menus([uint32]$ownedPid))[0]
}
function Cancel {
    for ($i=0;$i -lt 6;$i++) { if (-not [DarkMenuSmoke]::InMenu($window,[uint32]$ownedPid) -and @([DarkMenuSmoke]::Menus([uint32]$ownedPid)).Count -eq 0) { break }; Key 0x1B; Start-Sleep -Milliseconds 100 }
    Wait-For { -not [DarkMenuSmoke]::InMenu($window,[uint32]$ownedPid) -and @([DarkMenuSmoke]::Menus([uint32]$ownedPid)).Count -eq 0 }
    Start-Sleep -Milliseconds 100
}
function Check-Menu([bool]$Dark) {
    Guard; $root=[DarkMenuSmoke]::Root($window,[uint32]$ownedPid); $entries=@([DarkMenuSmoke]::Items($root))
    $labels=@('文件(&F)','代理(&P)','视图(&V)','帮助(&H)')
    $expected=@(@(0x3000,0x3010,0x3020,0x3030),@(0x3040,0x3050,0x3060),@(0x3070,0x3080,0x3090,0x30A0),@(0x30B0))
    for ($i=0;$i -lt 4;$i++) {
        if ($entries[$i].Text -cne $labels[$i] -or $entries[$i].Sub -eq [IntPtr]::Zero) { throw ('Root labels/submenu mismatch '+$i+': '+$entries[$i].Text) }
        $items=@([DarkMenuSmoke]::Items($entries[$i].Sub))
        if (($items.Id -join ',') -ne ($expected[$i] -join ',')) { throw 'Leaf commands changed' }
        foreach ($item in $items) { if ([string]::IsNullOrEmpty($item.Text) -or (($item.Type -band 0x100) -ne 0) -ne $Dark) { throw 'Leaf text/style mismatch' } }
    }
    foreach ($item in $entries) { if ((($item.Type -band 0x100) -ne 0) -ne $Dark) { throw 'Root native/dark style mismatch' } }
    foreach ($id in @(0xF120,0xF010,0xF000,0xF020,0xF030,0xF060)) { if (@($entries | Where-Object { $_.Id -eq $id }).Count -ne 1) { throw 'Native system command lost' } }
    $proxy=@([DarkMenuSmoke]::Items($entries[1].Sub))
    if (($proxy[0].State -band 3) -ne 0 -or ($proxy[1].State -band 3) -eq 0) { throw ('Start/stop availability wrong: '+($proxy | ConvertTo-Json -Compress)+'; buttons='+[DarkMenuSmoke]::IsWindowEnabled((C 2102))+','+[DarkMenuSmoke]::IsWindowEnabled((C 2103))) }
    $selection=[DarkMenuSmoke]::Message((C 2100),[uint32]$ownedPid,0x130B,0,0)
    $view=@([DarkMenuSmoke]::Items($entries[2].Sub))
    for ($i=0;$i -lt 4;$i++) { if ((($view[$i].State -band 8) -ne 0) -ne ($selection -eq $i)) { throw 'Radio state wrong' } }
    return $entries
}
function Capture([IntPtr]$Popup,[string]$Name,[bool]$Dark) {
    Guard; $shot=[DarkMenuSmoke]::Capture($Popup,[uint32]$ownedPid,(Join-Path $output ($Name+'.png')))
    $shots.Add($shot)
    if ($shot.SolidRgb) { throw "Invalid uniform popup capture: $Name RGB=$($shot.FirstRgb)" }
    if ($Dark -and ($shot.Dark -lt $shot.Pixels*0.75 -or $shot.Light -gt $shot.Pixels*0.12)) { throw "Popup still light/blank: $Name" }
    if (-not $Dark -and $shot.Light -lt $shot.Pixels*0.65) { throw "Native light fallback missing: $Name" }
}
function Select-Row([IntPtr]$Menu,[int]$Index) {
    for ($step=0;$step -lt 20;$step++) {
        $rows=@([DarkMenuSmoke]::Items($Menu)); $current=-1
        for ($j=0;$j -lt $rows.Count;$j++) { if (($rows[$j].State -band 0x80) -ne 0) { $current=$j } }
        if ($current -eq $Index) { return }
        $old=$current
        if ($current -lt 0 -or $current -lt $Index) { Key 0x28 } else { Key 0x26 }
        Wait-For { $rows=@([DarkMenuSmoke]::Items($Menu)); for ($j=0;$j -lt $rows.Count;$j++) { if (($rows[$j].State -band 0x80) -ne 0 -and $j -ne $old) { return $true } }; return $false }
    }
    throw 'Native arrow selection did not converge'
}
function Submenu([int]$Index) {
    $root=[DarkMenuSmoke]::Root($window,[uint32]$ownedPid)
    Start-Sleep -Milliseconds 100
    Select-Row $root $Index
    Key 0x27
    Wait-For { @([DarkMenuSmoke]::Menus([uint32]$ownedPid)).Count -eq 2 }
    return @([DarkMenuSmoke]::Items($root))[$Index].Sub
}
function Page([int]$Index) {
    Open | Out-Null; $sub=Submenu 2
    Select-Row $sub $Index
    Key 0x0D
    Wait-For { @([DarkMenuSmoke]::Menus([uint32]$ownedPid)).Count -eq 0 -and [DarkMenuSmoke]::Message((C 2100),[uint32]$ownedPid,0x130B,0,0) -eq $Index }
}
function Theme([int]$Index) {
    $control=C 2030
    [DarkMenuSmoke]::Message($control,[uint32]$ownedPid,0x14E,[uint64]$Index,0) | Out-Null
    [DarkMenuSmoke]::Post($window,[uint32]$ownedPid,0x111,[uint64](2030 -bor (1 -shl 16)),$control.ToInt64())
    Wait-For { $items=@([DarkMenuSmoke]::Items([DarkMenuSmoke]::Root($window,[uint32]$ownedPid))); (($items[0].Type -band 0x100) -ne 0) -eq ($Index -eq 2) }

}
No-Foreign
$mutex=$null
try { $mutex=[Threading.Mutex]::OpenExisting($mutexName) } catch [Threading.WaitHandleCannotBeOpenedException] {}
if ($null -ne $mutex) { $mutex.Dispose(); throw 'Existing user/session mutex; refused' }
try {
    $oldDpi=[DarkMenuSmoke]::Dpi([IntPtr](-4))
    $tag=[Guid]::NewGuid().ToString('N')
    $temp=Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-dark-menu-'+$tag)
    $output=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot ('..\target\dark-menu-ui\'+[DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')+'-'+$tag)))
    [void][IO.Directory]::CreateDirectory($temp); [void][IO.Directory]::CreateDirectory($output)
    $config=Join-Path $temp 'config.toml'
    $toml="schema_version=1`n[upstream]`nurl='http://127.0.0.1:1/v1'`n[proxy]`nauto_start=false`n[ui]`ntheme='light'`nremember_window_geometry=false`n[system]`nlogin_auto_start=false`n"
    [IO.File]::WriteAllText($config,$toml,[Text.UTF8Encoding]::new($false)); $hash=(Get-FileHash $config).Hash
    No-Foreign
    $stage='launch isolated stopped child'
    $child=Start-Process -FilePath $exe -WorkingDirectory $temp -ArgumentList @('--config-dir',('"'+$temp+'"')) -PassThru
    $ownedPid=$child.Id; $handle=$child.Handle
    Wait-For { $script:window=[DarkMenuSmoke]::FindMain([uint32]$ownedPid); $window -ne [IntPtr]::Zero } 10000
    $stage='wait native controls and background config load'
    Wait-For { $ready=C 2354; $ready -ne [IntPtr]::Zero -and [DarkMenuSmoke]::IsWindowEnabled((C 2102)) -and [DarkMenuSmoke]::IsWindowEnabled((C 2034)) -and [DarkMenuSmoke]::Text($ready,[uint32]$ownedPid).StartsWith('idle=1;') } 10000
    $dpi=[DarkMenuSmoke]::GetDpiForWindow($window)
    foreach ($index in @(1,2,1)) {
        $dark=$index -eq 2; $name=if($dark){'dark'}else{'light-'+$shots.Count}
        $stage='theme '+$name; Theme $index
        foreach ($method in @('F10','AltM','AltSpace')) {
            $popup=Open $method; Check-Menu $dark | Out-Null; Capture $popup ($name+'-'+$method) $dark; Cancel
        }
        for ($i=0;$i -lt 4;$i++) {
            $stage=$name+' submenu '+$i
            $popup=Open; Submenu $i | Out-Null; Check-Menu $dark | Out-Null
            $subPopup=@([DarkMenuSmoke]::Menus([uint32]$ownedPid) | Where-Object { $_ -ne $popup })[0]
            Capture $subPopup ($name+'-sub-'+$i) $dark; Cancel
        }
        Page 1; Page 3
    }
    $stage='dark mnemonic View / Overview'
    Theme 2; Open | Out-Null; Key 0x56
    Wait-For { @([DarkMenuSmoke]::Menus([uint32]$ownedPid)).Count -eq 2 }
    Key 0x4F
    Wait-For { @([DarkMenuSmoke]::Menus([uint32]$ownedPid)).Count -eq 0 -and [DarkMenuSmoke]::Message((C 2100),[uint32]$ownedPid,0x130B,0,0) -eq 0 }
    $stage='warm theme resource cycles'
    Theme 1; $baseline=[DarkMenuSmoke]::Resources($handle)
    for ($i=0;$i -lt 10;$i++) { Theme 2; Theme 1; $resources.Add([DarkMenuSmoke]::Resources($handle)) }
    if (($resources | Measure-Object -Maximum).Maximum -gt $baseline+2) { throw 'GDI resources grew after warmed cycles' }
    if ((Get-FileHash $config).Hash -ne $hash) { throw 'Config changed during menu/preview' }
    $stage='normal exit'; [DarkMenuSmoke]::Post($window,[uint32]$ownedPid,0x111,1002,0)
    if (-not $child.WaitForExit(15000) -or $child.ExitCode -ne 0) { throw 'Normal exit not proven' }
    $normalExit=$true; $configUnchanged=(Get-FileHash $config).Hash -eq $hash
    if (-not $configUnchanged) { throw 'Config changed on exit' }
} catch { $failure=$_.Exception.GetBaseException().Message; Write-Warning ($stage+': '+$failure); Write-Host $_.ScriptStackTrace; if($window -ne [IntPtr]::Zero) { Write-Host (([DarkMenuSmoke]::Items([DarkMenuSmoke]::Root($window,[uint32]$ownedPid))) | ConvertTo-Json -Compress) } }
finally {
    if ($null -ne $child -and -not $child.HasExited) {
        try {
            Guard -Cleanup
            if ($window -eq [IntPtr]::Zero) {
                $cleanupClock=[Diagnostics.Stopwatch]::StartNew()
                while ($window -eq [IntPtr]::Zero -and $cleanupClock.ElapsedMilliseconds -lt 5000) {
                    Guard -Cleanup
                    $window=[DarkMenuSmoke]::FindMain([uint32]$ownedPid)
                    if ($window -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 25 }
                }
            }
            [DarkMenuSmoke]::Post($window,[uint32]$ownedPid,0x1F,0,0)
            [DarkMenuSmoke]::Post($window,[uint32]$ownedPid,0x111,1002,0)
            if(-not $child.WaitForExit(15000)) { throw 'Exit deadline' }
        } catch { Write-Warning 'Normal cleanup unavailable; process and fixture retained, not killed' }
    }
    if ($oldDpi -ne [IntPtr]::Zero) { [DarkMenuSmoke]::Dpi($oldDpi) | Out-Null }
    if ($null -ne $output) {
        $report=[ordered]@{ AutomatedScopeComplete=($null -eq $failure -and $normalExit -and $configUnchanged); Complete=$false; Stage=$stage; Failure=$failure; Exe=$exe; PID=$ownedPid; DPI=$dpi; NormalExit=$normalExit; ConfigUnchanged=$configUnchanged; Screenshots=$shots.ToArray(); BaselineGdi=$baseline; GdiResources=$resources.ToArray(); Limits=@('OFFSCREEN PrintWindow, not physical visibility/input or pixel-perfect readability proof.','Actual desktop DPI only; high contrast, multi-monitor, screen reader and physical keyboard remain manual.','Title/system menu only; tray/context menus, dialogs and calendar are outside this fix.'); Temp=$temp }
        [IO.File]::WriteAllText((Join-Path $output 'report.json'),($report | ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
        Write-Host ('Evidence: '+$output)
    }
    if ($null -ne $temp -and $null -ne $child -and $child.HasExited -and $normalExit) { Remove-Item -LiteralPath $temp -Recurse -Force }
    if ($null -ne $child) { $child.Dispose() }
}
if ($null -ne $failure) { throw $failure }
'PASS: native dark title-menu scope; remaining acceptance in report.json'
