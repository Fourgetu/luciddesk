//! Structured folder columns; legacy metadata is consumed transactionally.
use super::*;
pub(super) const SCHEMA: &str = "CREATE TABLE panel_folder_view (
 panel_id INTEGER PRIMARY KEY REFERENCES panel_folder_settings(panel_id) ON DELETE CASCADE,
 name_width REAL, modified_width REAL, type_width REAL, size_width REAL,
 visible_columns INTEGER CHECK(visible_columns BETWEEN 1 AND 15 AND (visible_columns & 1)=1),
 CHECK((name_width IS NULL AND modified_width IS NULL AND type_width IS NULL AND size_width IS NULL)
 OR (name_width IS NOT NULL AND modified_width IS NOT NULL AND type_width IS NOT NULL AND size_width IS NOT NULL
 AND name_width>0 AND name_width<1 AND modified_width>0 AND modified_width<1
 AND type_width>0 AND type_width<1 AND size_width>0 AND size_width<1
 AND abs(name_width+modified_width+type_width+size_width-1)<0.001))
);";
pub(super) fn key(key: &str) -> Option<(&str, bool)> {
    key.strip_prefix("panel_folder_columns:")
        .map(|id| (id, false))
        .or_else(|| {
            key.strip_prefix("panel_folder_visible_columns:")
                .map(|id| (id, true))
        })
}
pub(super) fn read(db: &Connection, key_name: &str) -> Result<Option<String>, StoreError> {
    let Some((id, visible)) = key(key_name) else {
        return Ok(None);
    };
    if visible {
        return Ok(db
            .query_row(
                "SELECT visible_columns FROM panel_folder_view WHERE panel_id=?1",
                [id],
                |r| r.get::<_, Option<u8>>(0),
            )
            .optional()?
            .flatten()
            .map(|v| v.to_string()));
    }
    Ok(db.query_row("SELECT name_width,modified_width,type_width,size_width FROM panel_folder_view WHERE panel_id=?1 AND name_width IS NOT NULL",[id],|r|Ok([r.get::<_,f64>(0)?,r.get(1)?,r.get(2)?,r.get(3)?])).optional()?.map(|v|v.map(|v|format!("{v:.6}")).join(",")))
}
pub(super) fn write(db: &Connection, key_name: &str, value: &str) -> Result<bool, StoreError> {
    let Some((id, visible)) = key(key_name) else {
        return Ok(false);
    };
    if !db.query_row(
        "SELECT EXISTS(SELECT 1 FROM panel_folder_settings WHERE panel_id=?1)",
        [id],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(false);
    }
    let invalid = || StoreError::InvalidData("invalid folder column settings".into());
    if visible {
        let value = value
            .parse::<u8>()
            .ok()
            .filter(|v| *v <= 15 && v & 1 == 1)
            .ok_or_else(invalid)?;
        db.execute("INSERT INTO panel_folder_view(panel_id,visible_columns) VALUES (?1,?2) ON CONFLICT(panel_id) DO UPDATE SET visible_columns=excluded.visible_columns WHERE visible_columns IS NOT excluded.visible_columns",params![id,value])?;
    } else {
        let values: Vec<f64> = value
            .split(',')
            .map(str::parse)
            .collect::<Result<_, _>>()
            .map_err(|_| invalid())?;
        if values.len() != 4
            || values
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0 || *v >= 1.0)
            || (values.iter().sum::<f64>() - 1.0).abs() >= 0.001
        {
            return Err(invalid());
        }
        db.execute("INSERT INTO panel_folder_view(panel_id,name_width,modified_width,type_width,size_width) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(panel_id) DO UPDATE SET name_width=excluded.name_width,modified_width=excluded.modified_width,type_width=excluded.type_width,size_width=excluded.size_width WHERE (name_width,modified_width,type_width,size_width) IS NOT (excluded.name_width,excluded.modified_width,excluded.type_width,excluded.size_width)",params![id,values[0],values[1],values[2],values[3]])?;
    }
    Ok(true)
}
// Keep invalid legacy values untouched; the existing UI fallback still handles them.
pub(super) fn absorb(db: &Connection) -> Result<(), StoreError> {
    let rows:Vec<(String,String)>=db.prepare("SELECT key,value FROM metadata WHERE key LIKE 'panel_folder_columns:%' OR key LIKE 'panel_folder_visible_columns:%'")?.query_map([],|r|Ok((r.get(0)?,r.get(1)?)))?.collect::<Result<_,_>>()?;
    for (key, value) in rows {
        match write(db, &key, &value) {
            Ok(true) => {
                db.execute("DELETE FROM metadata WHERE key=?1", [key])?;
            }
            Ok(false) | Err(StoreError::InvalidData(_)) => (),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Persisted folder view preferences, independent of transient navigation and listing contents.
#[derive(Clone, Debug, PartialEq)]
pub struct FolderPreferences {
    pub sort_column: u8,
    pub descending: bool,
    pub column_widths: Option<[f32;4]>,
    pub visible_columns: u8,
}
impl Default for FolderPreferences {
    fn default() -> Self { Self { sort_column:0, descending:false, column_widths:None, visible_columns:15 } }
}
impl FolderPreferences {
    /// Validates the same limits used by the structured database tables.
    /// # Errors
    /// Rejects invalid sort columns, visibility masks and column proportions.
    pub fn validate(&self) -> Result<(),StoreError> {
        if self.sort_column>3 || self.visible_columns>15 || self.visible_columns&1==0
            || self.column_widths.is_some_and(|v|v.iter().any(|x|!x.is_finite()||*x<=0.0||*x>=1.0)||(v.iter().sum::<f32>()-1.0).abs()>=0.001) {
            return Err(StoreError::InvalidData("invalid folder view preferences".into()));
        }
        Ok(())
    }
}
pub(super) fn save_preferences(db:&Connection,id:PanelId,value:&FolderPreferences)->Result<(),StoreError> {
    value.validate()?;
    let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM panel_folder_settings WHERE panel_id=?1)",[id.get()],|r|r.get(0))?;
    if !exists { return Err(StoreError::InvalidData("folder panel does not exist".into())); }
    let direction=if value.descending {"desc"} else {"asc"};
    db.execute("UPDATE panel_folder_settings SET sort_column=?2,sort_direction=?3 WHERE panel_id=?1 AND (sort_column IS NOT ?2 OR sort_direction IS NOT ?3)",params![id.get(),value.sort_column,direction])?;
    let widths=value.column_widths.map(|v|v.map(f64::from));
    db.execute("INSERT INTO panel_folder_view(panel_id,name_width,modified_width,type_width,size_width,visible_columns) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(panel_id) DO UPDATE SET name_width=excluded.name_width,modified_width=excluded.modified_width,type_width=excluded.type_width,size_width=excluded.size_width,visible_columns=excluded.visible_columns WHERE (name_width,modified_width,type_width,size_width,visible_columns) IS NOT (excluded.name_width,excluded.modified_width,excluded.type_width,excluded.size_width,excluded.visible_columns)",params![id.get(),widths.map(|v|v[0]),widths.map(|v|v[1]),widths.map(|v|v[2]),widths.map(|v|v[3]),value.visible_columns])?;
    Ok(())
}
impl WorkspaceStore {
    /// Reads typed folder preferences. Missing optional columns use GUI defaults.
    /// # Errors
    /// Reports missing folder panels or database failures.
    pub fn folder_preferences(&self,id:PanelId)->Result<FolderPreferences,StoreError> {
        let (sort_column,direction):(u8,String)=self.connection.query_row("SELECT sort_column,sort_direction FROM panel_folder_settings WHERE panel_id=?1",[id.get()],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let values=self.connection.query_row("SELECT name_width,modified_width,type_width,size_width,visible_columns FROM panel_folder_view WHERE panel_id=?1",[id.get()],|r|Ok((r.get::<_,Option<f32>>(0)?,r.get::<_,Option<f32>>(1)?,r.get::<_,Option<f32>>(2)?,r.get::<_,Option<f32>>(3)?,r.get::<_,Option<u8>>(4)?))).optional()?;
        let (a,b,c,d,visible)=values.unwrap_or((None,None,None,None,None));
        let widths=match (a,b,c,d) {(Some(a),Some(b),Some(c),Some(d))=>Some([a,b,c,d]),_=>None};
        Ok(FolderPreferences{sort_column,descending:direction=="desc",column_widths:widths,visible_columns:visible.unwrap_or(15)})
    }
}
