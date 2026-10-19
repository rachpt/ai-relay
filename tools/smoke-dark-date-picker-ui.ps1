# Windows PowerShell 5.1 x64; private stopped fixture, no network/clipboard/Startup.
# Do not run concurrently with another GUI smoke/native HWND test.
[CmdletBinding()]
param([string]$ExePath,[switch]$ValidateOnly)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
# Reuse declarations only, never execute another smoke's body.
$helper=[IO.File]::ReadAllText((Join-Path $PSScriptRoot 'smoke-dark-menu-ui.ps1'))
$matches=[regex]::Matches($helper,'(?s)\$source = @''\r?\n(.*?)\r?\n''@')
if($matches.Count -ne 1) { throw 'Expected one isolated menu helper declaration' }
$menuSource=$matches[0].Groups[1].Value
$source=@'
using System;
using System.Runtime.InteropServices;
public static class DateTintNative {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int L,T,R,B; public int Width { get {return R-L;} } public int Height {get{return B-T;}} }
    [StructLayout(LayoutKind.Sequential)] public struct PickerInfo { public uint Size; public Rect Check; public uint CheckState; public Rect Button; public uint ButtonState; public IntPtr Edit,UpDown,DropDown; }
    [StructLayout(LayoutKind.Sequential)] struct GuiInfo {public uint Size,Flags;public IntPtr Active,Focus,Capture,MenuOwner,MoveSize,Caret;public Rect CaretRect;}
    [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access,bool inherit,uint pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll")] static extern uint GetProcessId(IntPtr handle);
    [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr handle,uint timeout);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode)] static extern bool QueryFullProcessImageNameW(IntPtr handle,uint flags,System.Text.StringBuilder path,ref uint capacity);
    public static void CheckProcess(IntPtr handle,uint pid,string exe){var path=new System.Text.StringBuilder(32768);uint size=(uint)path.Capacity;if(GetProcessId(handle)!=pid || WaitForSingleObject(handle,0)!=258 || !QueryFullProcessImageNameW(handle,0,path,ref size) || !String.Equals(path.ToString(),exe,StringComparison.OrdinalIgnoreCase))throw new Exception("Pinned executable identity mismatch");}
    [DllImport("kernel32.dll")] static extern IntPtr VirtualAllocEx(IntPtr process,IntPtr at,UIntPtr size,uint kind,uint protection);
    [DllImport("kernel32.dll")] static extern bool VirtualFreeEx(IntPtr process,IntPtr at,UIntPtr size,uint kind);
    [DllImport("kernel32.dll")] static extern bool WriteProcessMemory(IntPtr process,IntPtr at,byte[] bytes,UIntPtr size,out UIntPtr count);
    [DllImport("kernel32.dll")] static extern bool ReadProcessMemory(IntPtr process,IntPtr at,byte[] bytes,UIntPtr size,out UIntPtr count);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr h,uint m,UIntPtr w,IntPtr l,uint flags,uint timeout,out UIntPtr result);
    [DllImport("user32.dll",CharSet=CharSet.Unicode,EntryPoint="SendMessageTimeoutW")] static extern IntPtr SendText(IntPtr h,uint m,UIntPtr w,string l,uint flags,uint timeout,out UIntPtr result);
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h,out Rect r);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
    [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint thread,ref GuiInfo info);
    [DllImport("user32.dll")] static extern bool IsChild(IntPtr parent,IntPtr h);
    [StructLayout(LayoutKind.Sequential)] struct Point {public int X,Y;}
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h,out Rect r);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point point);
    static void Ink(System.Drawing.Bitmap image,string name,int left,int top,int right,int bottom){
        left=Math.Max(0,left);top=Math.Max(0,top);right=Math.Min(image.Width,right);bottom=Math.Min(image.Height,bottom);
        if(right-left<6 || bottom-top<4)throw new Exception("Content region too small: "+name);
        int[] bins=new int[16];
        Func<System.Drawing.Color,int> luma=delegate(System.Drawing.Color c){return (299*c.R+587*c.G+114*c.B)/1000;};
        for(int y=top;y<bottom;y++)for(int x=left;x<right;x++)bins[luma(image.GetPixel(x,y))/16]++;
        int mode=0;for(int i=1;i<bins.Length;i++)if(bins[i]>bins[mode])mode=i;
        int background=mode*16+8,ink=0,rows=0;
        for(int y=top;y<bottom;y++){
            int row=0;for(int x=left;x<right;x++)if(Math.Abs(luma(image.GetPixel(x,y))-background)>=40)row++;
            // Do not mistake a full-width separator/border for native text.
            if(row>=2 && row<(right-left)*.8){rows++;ink+=row;}
        }
        int area=(right-left)*(bottom-top);
        if(rows<3 || ink<Math.Max(6,area/500))throw new Exception("Missing native foreground content: "+name+" rows="+rows+" ink="+ink);
    }
    static void CalendarInk(System.Drawing.Bitmap b){
        Ink(b,"calendar title",b.Width*18/100,b.Height*2/100,b.Width*82/100,b.Height*17/100);
        Ink(b,"calendar day grid",b.Width*5/100,b.Height*27/100,b.Width*95/100,b.Height*86/100);
        Ink(b,"calendar today footer",b.Width*32/100,b.Height*88/100,b.Width*94/100,b.Height-2);
    }
    static void Foreground(uint pid,IntPtr h,System.Drawing.Bitmap b){
        string cls=DarkMenuSmoke.Class(h);
        if(cls=="SysMonthCal32")CalendarInk(b);
        else if(cls=="SysDateTimePick32"){
            var info=Info(pid,h);
            Ink(b,"picker date/empty text",info.Check.R+4,2,info.Button.L-3,b.Height-2);
        }else throw new Exception("Unexpected raster class: "+cls);
    }
    public static void Content(uint pid,IntPtr h,string path){DarkMenuSmoke.Check(h,pid);using(var b=new System.Drawing.Bitmap(path))Foreground(pid,h,b);}
    public static void ContentSelfTest(){
        using(var b=new System.Drawing.Bitmap(240,190))using(var graphics=System.Drawing.Graphics.FromImage(b))using(var font=new System.Drawing.Font("Segoe UI",10)){
            foreach(var color in new[]{System.Drawing.Color.White,System.Drawing.Color.FromArgb(31,38,50)}){
                graphics.Clear(color);bool rejected=false;try{CalendarInk(b);}catch{rejected=true;}if(!rejected)throw new Exception("Blank raster self-test accepted");
            }
            // Each missing region must fail independently, not just a flat image.
            for(int missing=-1;missing<3;missing++){
                graphics.Clear(System.Drawing.Color.White);
                if(missing!=0)graphics.DrawString("2024 February",font,System.Drawing.Brushes.Black,48,7);
                if(missing!=1)for(int row=0;row<5;row++)graphics.DrawString("12 13 14 15 16 17",font,System.Drawing.Brushes.Black,25,55+row*19);
                if(missing!=2)graphics.DrawString("Today: 2024/02/15",font,System.Drawing.Brushes.Black,76,171);
                bool rejected=false;try{CalendarInk(b);}catch{rejected=true;}
                if(rejected!=(missing>=0))throw new Exception("Regional content self-test mismatch");
            }
        }
    }
    public static string Screen(uint pid,IntPtr h,string path){
        DarkMenuSmoke.Check(h,pid);Rect r;if(!GetWindowRect(h,out r) || r.Width<8 || r.Height<8 || r.Width>4096 || r.Height>4096)throw new Exception("Physical bounds invalid");
        Func<bool> visible=delegate {foreach(int x in new int[]{r.L+2,(r.L+r.R)/2,r.R-3})foreach(int y in new int[]{r.T+2,(r.T+r.B)/2,r.B-3})if(DarkMenuSmoke.Owner(WindowFromPoint(new Point {X=x,Y=y}))!=pid)return false;return true;};
        if(!visible())return "SKIPPED: occluded/foreign window";
        using(var bitmap=new System.Drawing.Bitmap(r.Width,r.Height)){
            using(var graphics=System.Drawing.Graphics.FromImage(bitmap))graphics.CopyFromScreen(r.L,r.T,0,0,bitmap.Size);
            if(!visible())return "SKIPPED: occluded during capture";
            int dark=0,total=0;for(int y=3;y<r.Height-3;y++)for(int x=3;x<r.Width-3;x++){var c=bitmap.GetPixel(x,y);total++;if(c.R<110 && c.G<110 && c.B<110)dark++;}
            bitmap.Save(path,System.Drawing.Imaging.ImageFormat.Png);
            if(dark<total*.65)throw new Exception("Physical repaint remains white");Foreground(pid,h,bitmap);return "PASS: dark physical raster and native foreground regions";
        }
    }
    public static void Validate(){if(IntPtr.Size!=8 || Marshal.SizeOf(typeof(PickerInfo))!=72) throw new Exception("x64 PickerInfo ABI");}
    static byte[] Read(uint pid,IntPtr h,uint message,byte[] initial,out long result){
        DarkMenuSmoke.Check(h,pid); IntPtr process=OpenProcess(0x101038,false,pid); if(process==IntPtr.Zero) throw new Exception("Cannot open fixture VM");
        IntPtr remote=IntPtr.Zero; bool complete=true;
        try {
            var size=new UIntPtr((uint)initial.Length); remote=VirtualAllocEx(process,IntPtr.Zero,size,0x3000,4); if(remote==IntPtr.Zero) throw new Exception("Target buffer failed");
            UIntPtr count,value;
            if(!WriteProcessMemory(process,remote,initial,size,out count) || count.ToUInt64()!=(ulong)initial.Length) throw new Exception("Initialize target buffer failed");
            complete=false;
            if(SendMessageTimeoutW(h,message,UIntPtr.Zero,remote,0x22,1500,out value)==IntPtr.Zero) throw new Exception("Native query timed out; target buffer retained until process exit");
            complete=true; result=unchecked((long)value.ToUInt64()); var bytes=new byte[initial.Length];
            if(!ReadProcessMemory(process,remote,bytes,size,out count) || count.ToUInt64()!=(ulong)bytes.Length) throw new Exception("Read target buffer failed");
            return bytes;
        } finally { if(remote!=IntPtr.Zero && complete) VirtualFreeEx(process,remote,UIntPtr.Zero,0x8000); CloseHandle(process); }
    }
    public static PickerInfo Info(uint pid,IntPtr h){
        var bytes=new byte[72]; Array.Copy(BitConverter.GetBytes(72),bytes,4); long ignored; bytes=Read(pid,h,0x100E,bytes,out ignored);
        IntPtr local=Marshal.AllocHGlobal(72); try {Marshal.Copy(bytes,0,local,72);return (PickerInfo)Marshal.PtrToStructure(local,typeof(PickerInfo));} finally {Marshal.FreeHGlobal(local);}
    }
    public static string Date(uint pid,IntPtr h,bool calendar){
        long result;var bytes=Read(pid,h,0x1001,new byte[16],out result); if(!calendar && result==1)return "";
        if(calendar && result==0 || !calendar && result!=0)throw new Exception("Native date state failed");
        return new DateTime(BitConverter.ToUInt16(bytes,0),BitConverter.ToUInt16(bytes,2),BitConverter.ToUInt16(bytes,6)).ToString("yyyy-MM-dd",System.Globalization.CultureInfo.InvariantCulture);
    }
    public static Rect Minimum(uint pid,IntPtr h){long result;var b=Read(pid,h,0x1009,new byte[16],out result); if(result==0)throw new Exception("No calendar minimum");return new Rect {L=BitConverter.ToInt32(b,0),T=BitConverter.ToInt32(b,4),R=BitConverter.ToInt32(b,8),B=BitConverter.ToInt32(b,12)};}
    public static Rect Client(uint pid,IntPtr h){DarkMenuSmoke.Check(h,pid);Rect r;if(!GetClientRect(h,out r))throw new Exception("No client rect");return r;}
    public static void Set(uint pid,IntPtr h,string date){DarkMenuSmoke.Check(h,pid);UIntPtr result;if(SendText(h,12,UIntPtr.Zero,date,0x22,1500,out result)==IntPtr.Zero || result==UIntPtr.Zero)throw new Exception("Set ISO failed");}
    public static void Mouse(uint pid,IntPtr h,int x,int y){DarkMenuSmoke.Check(h,pid);var r=Client(pid,h);if(x<0 || x>=r.Width || y<0 || y>=r.Height)throw new Exception("Mouse bounds");long point=unchecked((int)(((uint)y<<16)|((uint)x&65535)));DarkMenuSmoke.Message(h,pid,0x200,0,point);DarkMenuSmoke.Message(h,pid,0x201,1,point);DarkMenuSmoke.Message(h,pid,0x202,0,point);}
    public static void Part(uint pid,IntPtr h,bool button){var info=Info(pid,h);var r=button?info.Button:info.Check;Mouse(pid,h,(r.L+r.R)/2,(r.T+r.B)/2);}
    public static bool Focused(uint pid,IntPtr h){DarkMenuSmoke.Check(h,pid);uint actual;var thread=GetWindowThreadProcessId(h,out actual);var info=new GuiInfo {Size=(uint)Marshal.SizeOf(typeof(GuiInfo))};if(!GetGUIThreadInfo(thread,ref info))throw new Exception("No focus info");return info.Focus==h || IsChild(h,info.Focus);}
}
'@
$joined=$menuSource+"`n"+$source
$pattern='(?m)^using [^;\r\n]+;\r?$'
$usings=@([regex]::Matches($joined,$pattern) | ForEach-Object {$_.Value.Trim()} | Select-Object -Unique)
Add-Type -TypeDefinition (($usings -join "`n")+"`n"+[regex]::Replace($joined,$pattern,'')) -ReferencedAssemblies System.dll,System.Core.dll,System.Drawing.dll
[DarkMenuSmoke]::Validate(); [DateTintNative]::Validate(); [DateTintNative]::ContentSelfTest()
if($ValidateOnly){'PASS: declarations/C# compile/x64 ABI and blank/regional-content rejection self-test; no GUI';return}
if([string]::IsNullOrWhiteSpace($ExePath)){$ExePath=Join-Path $PSScriptRoot '..\target\debug\ai-relay.exe'}
$exe=(Get-Item -LiteralPath $ExePath).FullName
$child=$null;$handle=[IntPtr]::Zero;$ownedPid=0;$window=[IntPtr]::Zero;$temp=$null;$output=$null;$failure=$null;$normalExit=$false;$oldDpi=[IntPtr]::Zero
$physical=[Collections.Generic.List[object]]::new();$gdi=[Collections.Generic.List[object]]::new();$shots=[Collections.Generic.List[object]]::new();$clock=[Diagnostics.Stopwatch]::StartNew();$stage='preflight';$dpi=0
function No-Foreign {if(@(Get-Process -Name @('ai-relay',[IO.Path]::GetFileNameWithoutExtension($exe)) -ErrorAction SilentlyContinue | Where-Object {$_.Id -ne $ownedPid}).Count -ne 0){throw 'Existing application; refused'}}
function Guard([switch]$Cleanup){No-Foreign;if(-not $Cleanup -and $clock.Elapsed.TotalSeconds -gt 150){throw '150s budget'};if($null -eq $child -or $child.HasExited -or $child.Handle -ne $handle){throw 'Pinned child not live'};[DateTintNative]::CheckProcess($handle,[uint32]$ownedPid,$exe)}
function Wait-For([scriptblock]$Test){$t=[Diagnostics.Stopwatch]::StartNew();while($t.ElapsedMilliseconds -lt 7000){Guard;if(& $Test){return};Start-Sleep -Milliseconds 30};throw ('Timeout: '+$stage)}
function C([int]$Id){Guard;[DarkMenuSmoke]::Control($window,[uint32]$ownedPid,$Id)}
function M([IntPtr]$H,[uint32]$Code,[uint64]$W=0,[long]$L=0){Guard;[DarkMenuSmoke]::Message($H,[uint32]$ownedPid,$Code,$W,$L)}
function Post([IntPtr]$H,[uint32]$Code,[uint64]$W=0,[long]$L=0){Guard;[DarkMenuSmoke]::Post($H,[uint32]$ownedPid,$Code,$W,$L)}
function Key([IntPtr]$H,[uint32]$Code){Guard;[DarkMenuSmoke]::Key($H,[uint32]$ownedPid,$Code,$false)}
function Page([int]$Index){Post $window 0x112 ([uint64](0x3070+16*$Index));Wait-For {(M (C 2100) 0x130B) -eq $Index}}
function Segment([int]$Id,[int]$Index){$h=C $Id;M $h 0x14E ([uint64]$Index) | Out-Null;Post $window 0x111 ([uint64]($Id -bor 0x10000)) $h.ToInt64();Wait-For {(M $h 0x130B) -eq $Index};Start-Sleep -Milliseconds 120}
function Theme([int]$Index){Segment 2030 $Index;Start-Sleep -Milliseconds 100}
function Date([int]$Id,[string]$Expected){$h=C $Id;Guard;$actual=[DateTintNative]::Date([uint32]$ownedPid,$h,$false);if($actual -cne $Expected -or [DarkMenuSmoke]::Text($h,[uint32]$ownedPid) -cne $Expected){throw ('Native/ISO date mismatch '+$Id+' actual='+$actual+' expected='+$Expected)}}
function Set-Date([int]$Id,[string]$ISO){Guard;[DateTintNative]::Set([uint32]$ownedPid,(C $Id),$ISO);Date $Id $ISO}
function Capture([IntPtr]$H,[string]$Name,[bool]$Dark){Guard;$shot=[DarkMenuSmoke]::Capture($H,[uint32]$ownedPid,(Join-Path $output ($Name+'.png')));$shots.Add($shot);[DateTintNative]::Content([uint32]$ownedPid,$H,$shot.Path);if($Dark -and ($shot.Dark -lt $shot.Pixels*.65 -or $shot.Light -gt $shot.Pixels*.12)){throw ('Still light/blank '+$Name)};if(-not $Dark -and $shot.Light -lt $shot.Pixels*.5){throw ('Native light fallback missing '+$Name)}}
function Screen([IntPtr]$H,[string]$Name){Guard;$path=Join-Path $output ($Name+'-physical.png');$result=[DateTintNative]::Screen([uint32]$ownedPid,$H,$path);$physical.Add(@{Name=$Name;Result=$result;Path=$path})}
function Open-Calendar([int]$Id){$h=C $Id;Guard;[DateTintNative]::Part([uint32]$ownedPid,$h,$true);Wait-For {(M $h 0x1008) -ne 0};$cal=[IntPtr](M $h 0x1008);Wait-For {[DarkMenuSmoke]::IsWindowVisible($cal)};Start-Sleep -Milliseconds 150;$min=[DateTintNative]::Minimum([uint32]$ownedPid,$cal);$r=[DateTintNative]::Client([uint32]$ownedPid,$cal);if($r.Width -lt $min.Width -or $r.Height -lt $min.Height){throw 'Calendar clipped under native minimum'};return $cal}
function Close-Calendar([int]$Id){$h=C $Id;$cal=[IntPtr](M $h 0x1008);if($cal -ne [IntPtr]::Zero){Key $cal 0x1B;Start-Sleep -Milliseconds 80;if((M $h 0x1008) -ne 0){M $h 0x100D | Out-Null};Wait-For {(M $h 0x1008) -eq 0}}}
No-Foreign
$session=[Diagnostics.Process]::GetCurrentProcess().SessionId;$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value;$mutex=$null
try{$mutex=[Threading.Mutex]::OpenExisting("Local\AIRelay.$sid.$session.v1")}catch [Threading.WaitHandleCannotBeOpenedException]{}
if($null -ne $mutex){$mutex.Dispose();throw 'Existing user/session mutex; refused'}
try{
    $oldDpi=[DarkMenuSmoke]::Dpi([IntPtr](-4));$tag=[Guid]::NewGuid().ToString('N');$temp=Join-Path ([IO.Path]::GetTempPath()) ('ai-relay-dark-date-'+$tag)
    $output=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot ('..\target\dark-date-ui\'+[DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')+'-'+$tag)))
    [void][IO.Directory]::CreateDirectory($temp);[void][IO.Directory]::CreateDirectory($output);$config=Join-Path $temp 'config.toml'
    [IO.File]::WriteAllText($config,"schema_version=1`n[upstream]`nurl='http://127.0.0.1:1/v1'`n[proxy]`nauto_start=false`n[ui]`ntheme='light'`nremember_window_geometry=false`n[system]`nlogin_auto_start=false`n",[Text.UTF8Encoding]::new($false));$hash=(Get-FileHash $config).Hash
    $stage='launch';$child=Start-Process -FilePath $exe -WorkingDirectory $temp -ArgumentList @('--config-dir',('"'+$temp+'"')) -PassThru;$handle=$child.Handle;$ownedPid=$child.Id
    Wait-For {$script:window=[DarkMenuSmoke]::FindMain([uint32]$ownedPid);$window -ne [IntPtr]::Zero};Wait-For {$r=C 2354;$r -ne [IntPtr]::Zero -and [DarkMenuSmoke]::Text($r,[uint32]$ownedPid).StartsWith('idle=1;')};$dpi=[DarkMenuSmoke]::GetDpiForWindow($window)
    foreach($theme in @(1,2,1)){
        $dark=$theme -eq 2;$name=if($dark){'dark'}else{'light-'+$shots.Count};$stage='theme '+$name;Theme $theme
        foreach($pair in @(@(2,2224,2225),@(1,2302,2303))){
            Page $pair[0];if($pair[0] -eq 1){Segment 2301 4}
            foreach($id in @($pair[1],$pair[2])){
                $stage=$name+' picker '+$id;Set-Date $id '2024-02-15';$h=C $id
                Capture $h ($name+'-'+$id+'-closed') $dark
                $info=[DateTintNative]::Info([uint32]$ownedPid,$h);[DateTintNative]::Mouse([uint32]$ownedPid,$h,($info.Check.R+24),([int](($info.Button.T+$info.Button.B)/2)))
                Wait-For {[DateTintNative]::Focused([uint32]$ownedPid,$h)};Capture $h ($name+'-'+$id+'-focused') $dark
                $cal=Open-Calendar $id;Capture $h ($name+'-'+$id+'-expanded') $dark;Capture $cal ($name+'-'+$id+'-calendar') $dark
                if($dark){Screen $h ($name+'-'+$id+'-expanded');Screen $cal ($name+'-'+$id+'-calendar')}
                Key $cal 0x22;Wait-For {[DateTintNative]::Date([uint32]$ownedPid,$cal,$true) -ceq '2024-03-15'}
                Key $cal 0x27;Wait-For {[DateTintNative]::Date([uint32]$ownedPid,$cal,$true) -ceq '2024-03-16'};Start-Sleep -Milliseconds 350;Capture $cal ($name+'-'+$id+'-calendar-after-nav') $dark;if($dark){Screen $cal ($name+'-'+$id+'-calendar-after-nav')};Key $cal 0x0D;Start-Sleep -Milliseconds 80
                if((M $h 0x1008) -ne 0){Key $h 0x0D};Close-Calendar $id;Date $id '2024-03-16';Capture $h ($name+'-'+$id+'-after') $dark
                [DateTintNative]::Part([uint32]$ownedPid,$h,$false);Date $id '';Capture $h ($name+'-'+$id+'-empty') $dark
            }
        }
    }
    for($cycle=0;$cycle -lt 8;$cycle++){Theme 2;Page 2;$cal=Open-Calendar 2225;Close-Calendar 2225;Theme 1;$gdi.Add([DarkMenuSmoke]::Resources($handle))}
    $late=@($gdi.ToArray() | Select-Object -Skip 2);if(($late | Measure-Object -Maximum).Maximum -gt ($late | Measure-Object -Minimum).Minimum+2){throw 'GDI growth after warm cycles'}
    if((Get-FileHash $config).Hash -ne $hash){throw 'Config changed'};$stage='normal exit';Post $window 0x111 1002
    if(-not $child.WaitForExit(15000) -or $child.ExitCode -ne 0){throw 'Normal exit failed'};$normalExit=$true;if((Get-FileHash $config).Hash -ne $hash){throw 'Exit changed config'}
}catch{$failure=$_.Exception.GetBaseException().Message;Write-Warning ($stage+': '+$failure);Write-Host $_.ScriptStackTrace}
finally{
    if($null -ne $child -and -not $child.HasExited){try{Guard -Cleanup;[DarkMenuSmoke]::Post($window,[uint32]$ownedPid,0x1F,0,0);[DarkMenuSmoke]::Post($window,[uint32]$ownedPid,0x111,1002,0);[void]$child.WaitForExit(15000)}catch{Write-Warning 'Fixture retained, not killed'}}
    if($oldDpi -ne [IntPtr]::Zero){[DarkMenuSmoke]::Dpi($oldDpi)|Out-Null}
    if($null -ne $output){$report=[ordered]@{Passed=($null -eq $failure -and $normalExit);Complete=$false;Failure=$failure;Stage=$stage;DPI=$dpi;NormalExit=$normalExit;Exe=$exe;Screenshots=$shots.ToArray();Physical=$physical.ToArray();GDI=$gdi.ToArray();Limits='OFFSCREEN native raster evidence and HWND input only; no real high contrast/multi-monitor/hardware-input/reader acceptance';Temp=$temp};[IO.File]::WriteAllText((Join-Path $output 'report.json'),($report|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false));Write-Host ('Evidence: '+$output)}
    if($null -ne $child){if($child.HasExited -and $normalExit){Remove-Item -LiteralPath $temp -Recurse -Force};$child.Dispose()}
}
if($null -ne $failure){throw $failure};'PASS: dark native date picker/calendar scope'
