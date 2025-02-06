// TODO: paste lib no longer maintained https://github.com/dtolnay/paste. update when found better alternative.
#[macro_export]
macro_rules! impl_ops_for_outpoint_data {
    ($prefix:expr) => {
        paste::paste! {
            pub async fn [<set_ $prefix:snake>](&self, outpoint: &OutPoint, data: &$prefix) {
                if !self.is_read_only {
                    self.database
                        .lock()
                        .await
                        .put([<$prefix:snake:upper _PREFIX>], &outpoint.to_string(), data);
                }
            }

            pub async fn [<delete_ $prefix:snake>](&self, outpoint: &OutPoint) {
                if !self.is_read_only {
                    self.database
                        .lock()
                        .await
                        .delete([<$prefix:snake:upper _PREFIX>], &outpoint.to_string());
                }
            }

            pub async fn [<get_ $prefix:snake>](&self, outpoint: &OutPoint) -> Result<$prefix, Flaw> {
                let data: Result<$prefix, DatabaseError> = self
                    .database
                    .lock()
                    .await
                    .get([<$prefix:snake:upper _PREFIX>], &outpoint.to_string());

                match data {
                    Ok(data) => Ok(data),
                    Err(DatabaseError::NotFound) => Ok($prefix::default()),
                    Err(DatabaseError::DeserializeFailed) => Err(Flaw::FailedDeserialization),
                }
            }
        }
    };
} 
