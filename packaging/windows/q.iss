#define MyAppName "q"
#define MyAppPublisher "Kirill Gorelov"
#define MyAppURL "https://github.com/kgorelov/q"
#define MyAppExeName "q.exe"
#define MyAppDaemonName "qdaemon.exe"
#ifndef MyAppVersion
  #define MyAppVersion "0.1.0"
#endif
#ifndef OutputBaseFilename
  #define OutputBaseFilename "q-" + MyAppVersion + "-windows-x64-setup"
#endif

[Setup]
AppId={{8A4E2F1B-9C3D-4E5F-B6A7-1D2E3F4A5B6C}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
AllowNoIcons=yes
OutputDir=..\..\dist
OutputBaseFilename={#OutputBaseFilename}
Compression=lzma2/ultra64
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\{#MyAppExeName}
ChangesEnvironment=yes

[Tasks]
Name: "addtopath"; Description: "Add q and schedule CLI utilities to system PATH"; GroupDescription: "Environment Configuration:"

[Files]
Source: "..\..\package\q\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Registry]
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; \
    ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; \
    Tasks: addtopath; Check: NeedsAddPath(ExpandConstant('{app}'))

[Code]
function NeedsAddPath(Param: string): boolean;
var
  OrigPath: string;
begin
  if not RegQueryStringValue(HKEY_LOCAL_MACHINE,
    'SYSTEM\CurrentControlSet\Control\Session Manager\Environment',
    'Path', OrigPath)
  then begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + UpperCase(Param) + ';', ';' + UpperCase(OrigPath) + ';') = 0;
end;
