// Included inside [Code]. Inspect targets, never infer ownership from a link name.
var
  UninstallShortcuts: TArrayOfString;
  ShortcutShell: Variant;

function ShortcutFileAttributes(Path: String): Cardinal;
  external 'GetFileAttributesW@kernel32.dll stdcall';
procedure NotifyShortcutPath(Event: Longint; Flags: Cardinal; Path: String; Unused: Longint);
  external 'SHChangeNotify@shell32.dll stdcall';
procedure NotifyShortcutIdList(Event: Longint; Flags: Cardinal; Item, Unused: LongWord);
  external 'SHChangeNotify@shell32.dll stdcall';
function ParseShortcutNamespace(Name: String; Context: LongWord; var Item: LongWord;
  Attributes: Cardinal; var ReturnedAttributes: Cardinal): Longint;
  external 'SHParseDisplayName@shell32.dll stdcall';
procedure FreeShortcutIdList(Item: LongWord);
  external 'CoTaskMemFree@ole32.dll stdcall';

function ShortcutTargetsThisInstall(Path: String): Boolean;
var
  Link: Variant;
  Target: String;
begin
  Result := False;
  try
    Link := ShortcutShell.CreateShortcut(Path);
    Target := Link.TargetPath;
    if Target = '' then Exit;
    Target := ExpandFileName(Target);
    Result := CompareText(Target, ExpandConstant('{app}\luciddesk.exe')) = 0;
  except
    Log('Skipping unreadable shortcut: ' + Path);
  end;
end;

procedure CollectShortcutDirectory(Directory: String; Recursive: Boolean; Depth: Integer);
var
  Found: TFindRec;
  Path: String;
  Index, Count: Integer;
  Attributes: Cardinal;
  Duplicate: Boolean;
begin
  if Depth > 16 then Exit;
  Attributes := ShortcutFileAttributes(Directory);
  if (Attributes = $FFFFFFFF) or ((Attributes and $400) <> 0) then Exit;
  if FindFirst(AddBackslash(Directory) + '*', Found) then begin
    try
      repeat
        if (Found.Name <> '.') and (Found.Name <> '..') and
           ((Found.Attributes and $400) = 0) then begin // Never traverse reparse points.
          Path := AddBackslash(Directory) + Found.Name;
          if (Found.Attributes and $10) <> 0 then begin
            if Recursive then CollectShortcutDirectory(Path, True, Depth + 1);
          end else if (CompareText(ExtractFileExt(Path), '.lnk') = 0) and
              ShortcutTargetsThisInstall(Path) then begin
            Duplicate := False;
            Count := GetArrayLength(UninstallShortcuts);
            for Index := 0 to Count - 1 do
              if CompareText(UninstallShortcuts[Index], Path) = 0 then Duplicate := True;
            if not Duplicate then begin
              SetArrayLength(UninstallShortcuts, Count + 1);
              UninstallShortcuts[Count] := Path;
            end;
          end;
        end;
      until not FindNext(Found);
    finally
      FindClose(Found);
    end;
  end;
end;

procedure CollectUninstallShortcuts;
begin
  try
    ShortcutShell := CreateOleObject('WScript.Shell');
    // Start Menu links belong to [Icons] and Inno's native uninstall log.
    CollectShortcutDirectory(ExpandConstant('{userdesktop}'), False, 0);
    if IsAdminInstallMode then begin
      CollectShortcutDirectory(ExpandConstant('{commondesktop}'), False, 0);
    end;
    Log(Format('Collected %d shortcuts targeting this installation.', [GetArrayLength(UninstallShortcuts)]));
  except
    Log('Shortcut inspection unavailable; continuing with Inno uninstall log cleanup.');
  end;
end;

procedure CleanupUninstallShortcuts;
var
  Index: Integer;
  Path: String;
  AppsFolder, Attributes: LongWord;
begin
  for Index := 0 to GetArrayLength(UninstallShortcuts) - 1 do begin
    Path := UninstallShortcuts[Index];
    if FileExists(Path) then begin
      // Check again: a shortcut edited during uninstall may now belong to another app.
      if not ShortcutTargetsThisInstall(Path) then Continue;
      if not DeleteFile(Path) then begin
        Log('Could not remove LucidDesk shortcut: ' + Path);
        Continue;
      end;
      Log('Removed shortcut targeting this installation: ' + Path);
    end;
    NotifyShortcutPath($4, $3005, Path, 0); // SHCNE_DELETE, PATHW | FLUSHNOWAIT
    if DirExists(ExtractFileDir(Path)) then
      NotifyShortcutPath($1000, $3005, ExtractFileDir(Path), 0); // SHCNE_UPDATEDIR
  end;
  NotifyShortcutPath($1000, $3005, ExpandConstant('{userprograms}'), 0);
  if IsAdminInstallMode then
    NotifyShortcutPath($1000, $3005, ExpandConstant('{commonprograms}'), 0);
  AppsFolder := 0;
  Attributes := 0;
  if ParseShortcutNamespace('shell:AppsFolder', 0, AppsFolder, 0, Attributes) = 0 then begin
    try
      NotifyShortcutIdList($1000, $3000, AppsFolder, 0);
    finally
      FreeShortcutIdList(AppsFolder);
    end;
  end;
  Log('Shell shortcut and AppsFolder refresh notifications sent.');
end;
