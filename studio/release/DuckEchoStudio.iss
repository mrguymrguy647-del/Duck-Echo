; Duck Echo Studio's Windows installer: DuckEchoStudioSetup.exe.
;
; Made with Inno Setup 6 (free, https://jrsoftware.org/isinfo.php) by the
; Studio workflow (.github/workflows/studio.yml), from the folder of
; ready-made programs it puts together:
;
;   ISCC.exe /DVersion=0.1.0 /DSource="dist\Duck Echo Studio" /O"dist" studio\release\DuckEchoStudio.iss
;
; What it does, as its siblings Duck Forge Studio and Duck Quill do:
; - Installs for you alone, with no administrator rights, in
;   %LOCALAPPDATA%\Programs\Duck Echo Studio: DuckEchoStudio.exe, and the
;   ready-made command-line tool and MCP server for AI assistants
;   (duck-echo.exe and duck-echo-mcp.exe). Nothing is built on your PC and
;   no Rust is needed.
; - Adds Duck Echo Studio to the Start menu and to Windows Settings > Apps,
;   to uninstall it. A desktop icon is a tick box, off at first (as in Duck
;   Quill).
; - "Add to PATH" (a tick box, off at first, as in Duck Quill) lets
;   terminals and AI assistants find duck-echo and duck-echo-mcp by name.
;   It changes only your own account's Path, and uninstalling takes it out.
; - Never touches your sounds or your settings. Your sounds and songs are in
;   Documents\Duck Echo, made by the Studio the first time it starts; your
;   settings are in %APPDATA%\Duck Echo Studio. Installing a newer version
;   over this one, or uninstalling, leaves both as they are.

#ifndef Version
  #define Version "0.0.0"
#endif
#ifndef Source
  #define Source "..\..\dist\Duck Echo Studio"
#endif
#ifndef SetupName
  #define SetupName "DuckEchoStudioSetup"
#endif

[Setup]
; The same id in every version, so a newer installer replaces an older
; install instead of adding a second one. Never change it.
AppId={{9FBE6DCB-D9A2-426D-A93C-163F40898E59}
AppName=Duck Echo Studio
AppVersion={#Version}
AppVerName=Duck Echo Studio {#Version}
AppPublisher=Duck Echo
AppPublisherURL=https://github.com/mrguymrguy647-del/Duck-Echo
AppSupportURL=https://github.com/mrguymrguy647-del/Duck-Echo
AppUpdatesURL=https://github.com/mrguymrguy647-del/Duck-Echo/releases
VersionInfoVersion={#Version}
; Just for you: no administrator rights, and {autopf} is then
; %LOCALAPPDATA%\Programs, which you can write to.
PrivilegesRequired=lowest
DefaultDirName={autopf}\Duck Echo Studio
; Not asked: a folder that needs administrator rights (like C:\Program
; Files) would not work.
DisableDirPage=yes
DisableProgramGroupPage=yes
OutputBaseFilename={#SetupName}
SetupIconFile=..\brand\duck-echo.ico
UninstallDisplayIcon={app}\DuckEchoStudio.exe
UninstallDisplayName=Duck Echo Studio
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; If the Studio is open (after pressing Update in it, say), close it first.
CloseApplications=yes
RestartApplications=no
; The "Add to PATH" box changes the Path: tell Windows, so new terminals
; see it.
ChangesEnvironment=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Put a Duck Echo Studio icon on the desktop"; GroupDescription: "Icons:"; Flags: unchecked
Name: "addtopath"; Description: "Let terminals and AI assistants find duck-echo and duck-echo-mcp (adds this folder to your Path)"; GroupDescription: "For AI assistants:"; Flags: unchecked

[Files]
Source: "{#Source}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Duck Echo Studio"; Filename: "{app}\DuckEchoStudio.exe"; WorkingDir: "{app}"; Comment: "Make sound effects and music without a terminal"
Name: "{autodesktop}\Duck Echo Studio"; Filename: "{app}\DuckEchoStudio.exe"; WorkingDir: "{app}"; Comment: "Make sound effects and music without a terminal"; Tasks: desktopicon

[Registry]
; Only your own account's Path (HKEY_CURRENT_USER), only if it is not
; there yet.
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Tasks: addtopath; Check: NotOnPath(ExpandConstant('{app}'))

[Run]
Filename: "{app}\DuckEchoStudio.exe"; Description: "Start Duck Echo Studio now"; Flags: nowait postinstall skipifsilent

[Code]
{ True when Folder is not one of the entries of your Path. }
function NotOnPath(Folder: String): Boolean;
var
  Path: String;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Path) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Uppercase(Folder) + ';', ';' + Uppercase(Path) + ';') = 0;
end;

{ Uninstalling takes the install folder out of your Path again, leaving
  every other entry as it was. }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Path, Folder: String;
  Place: Integer;
begin
  if CurUninstallStep <> usUninstall then
    exit;
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Path) then
    exit;
  Folder := ExpandConstant('{app}');
  Path := ';' + Path + ';';
  Place := Pos(';' + Uppercase(Folder) + ';', Uppercase(Path));
  if Place = 0 then
    exit;
  Delete(Path, Place, Length(Folder) + 1);
  { Take off the semicolons added above. }
  Path := Copy(Path, 2, Length(Path) - 2);
  RegWriteExpandStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Path);
end;
