use std::{fs::File, path::Path, sync::Arc};

use anyhow::Result;
use arrow::{
    array::{Int64Array, RecordBatch},
    datatypes::{DataType, Field, Schema},
};
use parquet::arrow::arrow_reader::ParquetRecordBatchReader;
use tempfile::NamedTempFile;

fn read_parquet(path: &Path) -> Result<()> {
    let file = File::open(path).unwrap();

    let parquet_reader = ParquetRecordBatchReader::try_new(file, 8)?;

    for batch in parquet_reader.take(1) {
        let batch = batch?;
        let ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        dbg!(ids);
    }

    Ok(())
}

fn write_parquet(file: &File) -> Result<()> {
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    let mut writer = parquet::arrow::ArrowWriter::try_new(file, schema.clone(), None)?;

    let ids = Arc::new(Int64Array::from(vec![69, 420]));

    let batch = RecordBatch::try_new(schema, vec![ids])?;
    writer.write(&batch)?;
    writer.close()?;

    Ok(())
}

fn main() -> Result<()> {
    let f = NamedTempFile::new()?;
    write_parquet(f.as_file())?;
    read_parquet(f.path())?;

    Ok(())
}
