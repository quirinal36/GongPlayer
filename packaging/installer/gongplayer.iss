; GitHub Release용 설치 프로그램 (Inno Setup 6). scripts/package-release.ps1이 컴파일합니다.
; - 관리자 권한 없이 현재 사용자에게만 설치합니다 (%LOCALAPPDATA%\Programs\GongPlayer).
; - 시작 메뉴 바로 가기와, 동영상 확장자의 "연결 프로그램" 목록 등록만 합니다. 기본 앱은 바꾸지 않습니다.
; - 서명하지 않으므로 SmartScreen 경고가 나옵니다 (docs/DISTRIBUTION.md).

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef StageDir
  #define StageDir "..\..\dist\release\stage"
#endif
#ifndef OutDir
  #define OutDir "..\..\dist\release"
#endif

#define AppName "GongPlayer"
#define ProgId "GongPlayer.Video"

[Setup]
; AppId는 바꾸지 않습니다. 바꾸면 업데이트가 아니라 별개의 앱으로 설치됩니다.
AppId={{6F1C2E7A-4B3D-4A8E-9C51-2D7B0E9F4A10}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=letscoding
AppPublisherURL=https://github.com/quirinal36/GongPlayer
AppSupportURL=https://github.com/quirinal36/GongPlayer/issues
AppUpdatesURL=https://github.com/quirinal36/GongPlayer/releases
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19041
OutputDir={#OutDir}
OutputBaseFilename={#AppName}-{#AppVersion}-setup
SetupIconFile=..\gongplayer.ico
UninstallDisplayIcon={app}\GongPlayer.exe
UninstallDisplayName={#AppName}
LicenseFile={#StageDir}\LICENSE.txt
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes
CloseApplications=yes
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}

[Languages]
Name: "korean"; MessagesFile: "compiler:Languages\Korean.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#StageDir}\GongPlayer.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\FFMPEG_LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\DAV1D_LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\GongPlayer.exe"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\GongPlayer.exe"; Tasks: desktopicon

[Registry]
; 탐색기의 "연결 프로그램"에 나오게 합니다. 기본 앱 선택은 사용자에게 맡깁니다.
Root: HKA; Subkey: "Software\Classes\{#ProgId}"; ValueType: string; ValueName: ""; ValueData: "{#AppName} 동영상"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\{#ProgId}\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\GongPlayer.exe,0"
Root: HKA; Subkey: "Software\Classes\{#ProgId}\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\GongPlayer.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\GongPlayer.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "{#AppName}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\GongPlayer.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\GongPlayer.exe"" ""%1"""
#define Ext(e) \
  "Root: HKA; Subkey: ""Software\Classes\" + e + "\OpenWithProgids""; ValueType: string; ValueName: """ + ProgId + """; ValueData: """"; Flags: uninsdeletevalue" + NewLine + \
  "Root: HKA; Subkey: ""Software\Classes\Applications\GongPlayer.exe\SupportedTypes""; ValueType: string; ValueName: """ + e + """; ValueData: """""
{#Ext(".mp4")}
{#Ext(".m4v")}
{#Ext(".mov")}
{#Ext(".mkv")}
{#Ext(".webm")}
{#Ext(".ts")}
{#Ext(".m2ts")}
{#Ext(".mts")}
{#Ext(".avi")}

[Run]
Filename: "{app}\GongPlayer.exe"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
