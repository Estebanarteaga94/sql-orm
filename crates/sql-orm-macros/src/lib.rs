use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use std::collections::BTreeMap;
use syn::{
    Data, DeriveInput, Error, Expr, ExprLit, Field, Fields, Ident, Lit, LitStr, Path, Result,
    Token, Type, parse_macro_input, punctuated::Punctuated, spanned::Spanned,
};

#[proc_macro_derive(Entity, attributes(orm))]
pub fn derive_entity(input: TokenStream) -> TokenStream {
    match derive_entity_impl(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_derive(DbContext, attributes(orm))]
pub fn derive_db_context(input: TokenStream) -> TokenStream {
    match derive_db_context_impl(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_derive(Insertable, attributes(orm))]
pub fn derive_insertable(input: TokenStream) -> TokenStream {
    match derive_insertable_impl(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_derive(Changeset, attributes(orm))]
pub fn derive_changeset(input: TokenStream) -> TokenStream {
    match derive_changeset_impl(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_derive(FromRow, attributes(orm))]
pub fn derive_from_row(input: TokenStream) -> TokenStream {
    match derive_from_row_impl(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_derive(AuditFields, attributes(orm))]
pub fn derive_audit_fields(input: TokenStream) -> TokenStream {
    match derive_policy_fields_impl(
        parse_macro_input!(input as DeriveInput),
        PolicyFieldsKind::Audit,
    ) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_derive(SoftDeleteFields, attributes(orm))]
pub fn derive_soft_delete_fields(input: TokenStream) -> TokenStream {
    match derive_policy_fields_impl(
        parse_macro_input!(input as DeriveInput),
        PolicyFieldsKind::SoftDelete,
    ) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_derive(TenantContext, attributes(orm))]
pub fn derive_tenant_context(input: TokenStream) -> TokenStream {
    match derive_tenant_context_impl(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[derive(Clone, Copy)]
enum PolicyFieldsKind {
    Audit,
    SoftDelete,
}

impl PolicyFieldsKind {
    fn derive_name(self) -> &'static str {
        match self {
            Self::Audit => "AuditFields",
            Self::SoftDelete => "SoftDeleteFields",
        }
    }

    fn policy_name(self) -> &'static str {
        match self {
            Self::Audit => "audit",
            Self::SoftDelete => "soft_delete",
        }
    }

    fn default_insertable(self) -> bool {
        match self {
            Self::Audit => true,
            Self::SoftDelete => false,
        }
    }

    fn default_updatable(self) -> bool {
        true
    }
}

fn derive_policy_fields_impl(input: DeriveInput, kind: PolicyFieldsKind) -> Result<TokenStream2> {
    let ident = input.ident;
    let derive_name = kind.derive_name();
    let policy_name = kind.policy_name();
    let fields = match input.data {
        Data::Struct(data) => match data.fields {
            Fields::Named(fields) => fields.named,
            _ => {
                return Err(Error::new_spanned(
                    ident,
                    format!("{derive_name} solo soporta structs con campos nombrados"),
                ));
            }
        },
        _ => {
            return Err(Error::new_spanned(
                ident,
                format!("{derive_name} solo soporta structs"),
            ));
        }
    };

    let mut columns = Vec::new();
    let mut column_names = Vec::new();
    let mut runtime_values = Vec::new();
    let mut seen_column_names = std::collections::BTreeSet::new();

    for field in fields.iter() {
        let field_ident = field.ident.as_ref().ok_or_else(|| {
            Error::new_spanned(field, format!("{derive_name} requiere campos nombrados"))
        })?;
        let config = parse_policy_field_config(field, kind)?;
        let type_info = analyze_type(&field.ty)?;
        let field_ty = &field.ty;
        let rust_field = LitStr::new(&field_ident.to_string(), field_ident.span());
        let column_name = config
            .column
            .unwrap_or_else(|| LitStr::new(&field_ident.to_string(), field_ident.span()));
        validate_non_empty_lit_str(&column_name, "column no puede estar vacío")?;
        if !seen_column_names.insert(column_name.value()) {
            return Err(Error::new_spanned(
                &column_name,
                format!("{derive_name} no permite columnas duplicadas"),
            ));
        }
        column_names.push(column_name.clone());
        let renamed_from = option_lit_str(config.renamed_from);
        let sql_type = match (config.sql_type, config.unsafe_sql_type) {
            (Some(sql_type), None) => sql_type_from_string(&sql_type)?,
            (None, Some(sql_type)) => unsafe_sql_type_from_string(&sql_type),
            (Some(sql_type), Some(_)) => {
                return Err(Error::new_spanned(
                    sql_type,
                    "sql_type y unsafe_sql_type no pueden usarse juntos",
                ));
            }
            (None, None) => {
                quote! { <#field_ty as ::sql_orm::core::SqlTypeMapping>::SQL_SERVER_TYPE }
            }
        };
        let nullable = config.nullable || type_info.nullable;
        let default_sql = option_lit_str(config.default_sql);
        let max_length = config.length.map_or_else(
            || quote! { <#field_ty as ::sql_orm::core::SqlTypeMapping>::DEFAULT_MAX_LENGTH },
            |length| quote! { Some(#length) },
        );
        let precision = config.precision.map_or_else(
            || quote! { <#field_ty as ::sql_orm::core::SqlTypeMapping>::DEFAULT_PRECISION },
            |precision| quote! { Some(#precision) },
        );
        let scale = config.scale.map_or_else(
            || quote! { <#field_ty as ::sql_orm::core::SqlTypeMapping>::DEFAULT_SCALE },
            |scale| quote! { Some(#scale) },
        );
        let insertable = config.insertable.unwrap_or(kind.default_insertable());
        let updatable = config.updatable.unwrap_or(kind.default_updatable());

        if matches!(kind, PolicyFieldsKind::Audit | PolicyFieldsKind::SoftDelete) {
            runtime_values.push(quote! {
                ::sql_orm::core::ColumnValue::new(
                    #column_name,
                    <#field_ty as ::sql_orm::core::SqlTypeMapping>::to_sql_value(
                        self.#field_ident
                    )
                )
            });
        }

        columns.push(quote! {
            ::sql_orm::core::ColumnMetadata {
                rust_field: #rust_field,
                column_name: #column_name,
                renamed_from: #renamed_from,
                sql_type: #sql_type,
                nullable: #nullable,
                primary_key: false,
                identity: None,
                default_sql: #default_sql,
                computed_sql: None,
                rowversion: false,
                insertable: #insertable,
                updatable: #updatable,
                max_length: #max_length,
                precision: #precision,
                scale: #scale,
            }
        });
    }

    let runtime_values_impl = match kind {
        PolicyFieldsKind::Audit => quote! {
            impl ::sql_orm::AuditValues for #ident {
                fn audit_values(self) -> Vec<::sql_orm::core::ColumnValue> {
                    vec![
                        #(#runtime_values),*
                    ]
                }
            }
        },
        PolicyFieldsKind::SoftDelete => quote! {
            impl ::sql_orm::SoftDeleteValues for #ident {
                fn soft_delete_values(self) -> Vec<::sql_orm::core::ColumnValue> {
                    vec![
                        #(#runtime_values),*
                    ]
                }
            }
        },
    };

    Ok(quote! {
        impl ::sql_orm::core::EntityPolicy for #ident {
            const POLICY_NAME: &'static str = #policy_name;
            const COLUMN_NAMES: &'static [&'static str] = &[#(#column_names),*];

            fn columns() -> &'static [::sql_orm::core::ColumnMetadata] {
                const COLUMNS: &[::sql_orm::core::ColumnMetadata] = &[
                    #(#columns),*
                ];

                COLUMNS
            }
        }

        #runtime_values_impl
    })
}

fn derive_tenant_context_impl(input: DeriveInput) -> Result<TokenStream2> {
    let ident = input.ident;
    let fields = match input.data {
        Data::Struct(data) => match data.fields {
            Fields::Named(fields) => fields.named,
            _ => {
                return Err(Error::new_spanned(
                    ident,
                    "TenantContext solo soporta structs con campos nombrados",
                ));
            }
        },
        _ => {
            return Err(Error::new_spanned(
                ident,
                "TenantContext solo soporta structs",
            ));
        }
    };

    if fields.len() != 1 {
        return Err(Error::new_spanned(
            ident,
            "TenantContext requiere exactamente un campo tenant",
        ));
    }

    let field = fields
        .first()
        .expect("TenantContext must have exactly one field after validation");
    let field_ident = field
        .ident
        .as_ref()
        .ok_or_else(|| Error::new_spanned(field, "TenantContext requiere campos nombrados"))?;
    let config = parse_tenant_context_field_config(field)?;
    let type_info = analyze_type(&field.ty)?;

    if type_info.nullable {
        return Err(Error::new_spanned(
            &field.ty,
            "TenantContext no soporta Option<T>; la ausencia de tenant debe representarse sin configurar tenant activo en el contexto",
        ));
    }

    let field_ty = &field.ty;
    let rust_field = LitStr::new(&field_ident.to_string(), field_ident.span());
    let column_name = config
        .column
        .unwrap_or_else(|| LitStr::new(&field_ident.to_string(), field_ident.span()));
    validate_non_empty_lit_str(&column_name, "column no puede estar vacío")?;
    let renamed_from = option_lit_str(config.renamed_from);
    let sql_type = match (config.sql_type, config.unsafe_sql_type) {
        (Some(sql_type), None) => sql_type_from_string(&sql_type)?,
        (None, Some(sql_type)) => unsafe_sql_type_from_string(&sql_type),
        (Some(sql_type), Some(_)) => {
            return Err(Error::new_spanned(
                sql_type,
                "sql_type y unsafe_sql_type no pueden usarse juntos",
            ));
        }
        (None, None) => {
            quote! { <#field_ty as ::sql_orm::core::SqlTypeMapping>::SQL_SERVER_TYPE }
        }
    };
    let max_length = config.length.map_or_else(
        || quote! { <#field_ty as ::sql_orm::core::SqlTypeMapping>::DEFAULT_MAX_LENGTH },
        |length| quote! { Some(#length) },
    );
    let precision = config.precision.map_or_else(
        || quote! { <#field_ty as ::sql_orm::core::SqlTypeMapping>::DEFAULT_PRECISION },
        |precision| quote! { Some(#precision) },
    );
    let scale = config.scale.map_or_else(
        || quote! { <#field_ty as ::sql_orm::core::SqlTypeMapping>::DEFAULT_SCALE },
        |scale| quote! { Some(#scale) },
    );

    Ok(quote! {
        impl ::sql_orm::core::EntityPolicy for #ident {
            const POLICY_NAME: &'static str = "tenant";
            const COLUMN_NAMES: &'static [&'static str] = &[#column_name];

            fn columns() -> &'static [::sql_orm::core::ColumnMetadata] {
                const COLUMNS: &[::sql_orm::core::ColumnMetadata] = &[
                    ::sql_orm::core::ColumnMetadata {
                        rust_field: #rust_field,
                        column_name: #column_name,
                        renamed_from: #renamed_from,
                        sql_type: #sql_type,
                        nullable: false,
                        primary_key: false,
                        identity: None,
                        default_sql: None,
                        computed_sql: None,
                        rowversion: false,
                        insertable: true,
                        updatable: false,
                        max_length: #max_length,
                        precision: #precision,
                        scale: #scale,
                    }
                ];

                COLUMNS
            }
        }

        impl ::sql_orm::TenantContext for #ident {
            const COLUMN_NAME: &'static str = #column_name;

            fn tenant_value(&self) -> ::sql_orm::core::SqlValue {
                <#field_ty as ::sql_orm::core::SqlTypeMapping>::to_sql_value(
                    ::core::clone::Clone::clone(&self.#field_ident)
                )
            }
        }
    })
}

fn derive_from_row_impl(input: DeriveInput) -> Result<TokenStream2> {
    let ident = input.ident;
    let fields = extract_named_fields(&ident, input.data, "FromRow")?;
    let generics = input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let field_initializers = fields
        .iter()
        .map(|field| {
            let field_ident = field
                .ident
                .as_ref()
                .ok_or_else(|| Error::new_spanned(field, "FromRow requiere campos nombrados"))?;
            let config = parse_persistence_field_config(field, "FromRow")?;
            let column_name = config
                .column
                .unwrap_or_else(|| LitStr::new(&field_ident.to_string(), field_ident.span()));
            validate_non_empty_lit_str(&column_name, "column no puede estar vacío")?;

            let field_ty = &field.ty;
            let type_info = analyze_type(field_ty)?;
            let value = if type_info.nullable {
                quote! {
                    row.try_get_typed::<#field_ty>(#column_name)?.flatten()
                }
            } else {
                quote! {
                    row.get_required_typed::<#field_ty>(#column_name)?
                }
            };

            Ok(quote! {
                #field_ident: #value
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(quote! {
        impl #impl_generics ::sql_orm::core::FromRow for #ident #ty_generics #where_clause {
            fn from_row<R: ::sql_orm::core::Row>(row: &R) -> Result<Self, ::sql_orm::core::OrmError> {
                Ok(Self {
                    #(#field_initializers),*
                })
            }
        }
    })
}

fn derive_entity_impl(input: DeriveInput) -> Result<TokenStream2> {
    let ident = input.ident;
    let EntityConfig {
        table: entity_table,
        schema: entity_schema,
        renamed_from: entity_renamed_from,
        indexes: entity_indexes,
        audit: entity_audit,
        soft_delete: entity_soft_delete,
        tenant: entity_tenant,
    } = parse_entity_config(&input.attrs)?;
    let fields = match input.data {
        Data::Struct(data) => match data.fields {
            Fields::Named(fields) => fields.named,
            _ => {
                return Err(Error::new_spanned(
                    ident,
                    "Entity solo soporta structs con campos nombrados",
                ));
            }
        },
        _ => return Err(Error::new_spanned(ident, "Entity solo soporta structs")),
    };

    let schema = entity_schema.unwrap_or_else(|| LitStr::new("dbo", Span::call_site()));
    let table =
        entity_table.unwrap_or_else(|| LitStr::new(&default_table_name(&ident), ident.span()));
    let renamed_from = option_lit_str(entity_renamed_from);
    let rust_name = LitStr::new(&ident.to_string(), ident.span());

    let mut columns = Vec::new();
    let mut column_symbols = Vec::new();
    let mut primary_key_columns = Vec::new();
    let mut primary_key_value_expr = None;
    let mut persist_mode_expr = None;
    let mut insert_values = Vec::new();
    let mut update_changes = Vec::new();
    let mut entity_concurrency_token = None;
    let mut sync_fields = Vec::new();
    let mut from_row_fields = Vec::new();
    let mut indexes = Vec::new();
    let mut foreign_keys = Vec::new();
    let mut foreign_key_accessors = Vec::new();
    let mut field_foreign_keys = BTreeMap::<String, FieldForeignKeyInfo>::new();
    let mut navigations = Vec::new();
    let mut field_columns = BTreeMap::<String, LitStr>::new();
    let mut entity_column_names = Vec::new();

    let has_explicit_primary_key = has_explicit_primary_key(&fields)?;

    for field in fields.iter() {
        let field_ident = field
            .ident
            .as_ref()
            .ok_or_else(|| Error::new_spanned(field, "Entity requiere campos nombrados"))?;
        let config = parse_field_config(field)?;
        let rust_field = LitStr::new(&field_ident.to_string(), field_ident.span());

        if let Some(navigation) = config.navigation {
            let wrapper = validate_navigation_field_type(&field.ty, &navigation)?;
            let target = navigation.target.clone();
            let target_rust_name = LitStr::new(
                &path_last_ident(&target)
                    .map(|ident| ident.to_string())
                    .unwrap_or_else(|| quote! { #target }.to_string()),
                target.span(),
            );
            let kind = navigation_kind_tokens(navigation.kind);
            let foreign_key_field = navigation.foreign_key.clone();
            let foreign_key_field_name = foreign_key_field.to_string();

            from_row_fields.push(quote! {
                #field_ident: ::core::default::Default::default()
            });
            sync_fields.push(quote! {
                self.#field_ident = persisted.#field_ident;
            });

            navigations.push(PendingNavigation {
                rust_field,
                kind: navigation.kind,
                kind_tokens: kind,
                wrapper,
                target,
                target_rust_name,
                foreign_key_field,
                foreign_key_field_name,
            });

            continue;
        }

        let column_name = config
            .column
            .unwrap_or_else(|| LitStr::new(&field_ident.to_string(), field_ident.span()));
        entity_column_names.push(column_name.clone());
        field_columns.insert(field_ident.to_string(), column_name.clone());
        let type_info = analyze_type(&field.ty)?;

        let primary_key = config.primary_key
            || (field_ident == &Ident::new("id", field_ident.span()) && !has_explicit_primary_key);
        if primary_key {
            primary_key_columns.push(column_name.clone());
            let field_ty = &field.ty;
            if primary_key_value_expr.is_none() {
                primary_key_value_expr = Some(quote! {
                    Ok(<#field_ty as ::sql_orm::core::SqlTypeMapping>::to_sql_value(
                        ::core::clone::Clone::clone(&self.#field_ident)
                    ))
                });
            }

            if persist_mode_expr.is_none() {
                let identity_strategy = if config.identity {
                    match type_info.kind {
                        TypeKind::I64 | TypeKind::I32 | TypeKind::I16 | TypeKind::U8 => {
                            Some(quote! {
                                if self.#field_ident == 0 {
                                    Ok(::sql_orm::EntityPersistMode::Insert)
                                } else {
                                    Ok(::sql_orm::EntityPersistMode::Update(
                                        <#field_ty as ::sql_orm::core::SqlTypeMapping>::to_sql_value(
                                            ::core::clone::Clone::clone(&self.#field_ident)
                                        )
                                    ))
                                }
                            })
                        }
                        _ => None,
                    }
                } else {
                    None
                };

                persist_mode_expr = Some(identity_strategy.unwrap_or_else(|| {
                    quote! {
                        Ok(::sql_orm::EntityPersistMode::InsertOrUpdate(
                            <#field_ty as ::sql_orm::core::SqlTypeMapping>::to_sql_value(
                                ::core::clone::Clone::clone(&self.#field_ident)
                            )
                        ))
                    }
                }));
            }
        }

        let sql_type = match (config.sql_type, config.unsafe_sql_type) {
            (Some(sql_type), None) => sql_type_from_string(&sql_type)?,
            (None, Some(sql_type)) => unsafe_sql_type_from_string(&sql_type),
            (Some(sql_type), Some(_)) => {
                return Err(Error::new_spanned(
                    sql_type,
                    "sql_type y unsafe_sql_type no pueden usarse juntos",
                ));
            }
            (None, None) => infer_sql_type(&type_info, config.rowversion, &field.ty)?,
        };

        if config.identity && !type_info.is_integer {
            return Err(Error::new_spanned(
                &field.ty,
                "identity solo se soporta sobre tipos enteros",
            ));
        }

        if config.rowversion && !type_info.is_vec_u8 {
            return Err(Error::new_spanned(&field.ty, "rowversion requiere Vec<u8>"));
        }

        let nullable = config.nullable || type_info.nullable;
        let identity = if config.identity {
            let seed = config.identity_seed.unwrap_or(1);
            let increment = config.identity_increment.unwrap_or(1);
            quote! {
                Some(::sql_orm::core::IdentityMetadata::new(#seed, #increment))
            }
        } else {
            quote! { None }
        };

        let max_length = config
            .length
            .or_else(|| type_info.default_max_length.filter(|_| !config.rowversion));
        let precision = config.precision.or(type_info.default_precision);
        let scale = config.scale.or(type_info.default_scale);
        let default_sql = option_lit_str(config.default_sql);
        let renamed_from = option_lit_str(config.renamed_from);
        let has_computed_sql = config.computed_sql.is_some();
        let computed_sql = option_lit_str(config.computed_sql);
        let max_length = option_number(max_length);
        let precision = option_number(precision);
        let scale = option_number(scale);
        let rowversion = config.rowversion;
        let insertable = !config.identity && !rowversion && !has_computed_sql;
        let updatable = !primary_key && !rowversion && !has_computed_sql;

        columns.push(quote! {
            ::sql_orm::core::ColumnMetadata {
                rust_field: #rust_field,
                column_name: #column_name,
                renamed_from: #renamed_from,
                sql_type: #sql_type,
                nullable: #nullable,
                primary_key: #primary_key,
                identity: #identity,
                default_sql: #default_sql,
                computed_sql: #computed_sql,
                rowversion: #rowversion,
                insertable: #insertable,
                updatable: #updatable,
                max_length: #max_length,
                precision: #precision,
                scale: #scale,
            }
        });

        column_symbols.push(quote! {
            pub const #field_ident: ::sql_orm::core::EntityColumn<#ident> =
                ::sql_orm::core::EntityColumn::new(#rust_field, #column_name);
        });

        if insertable {
            let field_ty = &field.ty;
            insert_values.push(quote! {
                values.push(::sql_orm::core::ColumnValue::new(
                    #column_name,
                    <#field_ty as ::sql_orm::core::SqlTypeMapping>::to_sql_value(
                        ::core::clone::Clone::clone(&self.#field_ident)
                    ),
                ));
            });
        }

        if updatable {
            let field_ty = &field.ty;
            update_changes.push(quote! {
                changes.push(::sql_orm::core::ColumnValue::new(
                    #column_name,
                    <#field_ty as ::sql_orm::core::SqlTypeMapping>::to_sql_value(
                        ::core::clone::Clone::clone(&self.#field_ident)
                    ),
                ));
            });
        }

        if rowversion {
            let field_ty = &field.ty;
            entity_concurrency_token = Some(quote! {
                Ok(Some(
                    <#field_ty as ::sql_orm::core::SqlTypeMapping>::to_sql_value(
                        ::core::clone::Clone::clone(&self.#field_ident)
                    )
                ))
            });
        }

        sync_fields.push(quote! {
            self.#field_ident = persisted.#field_ident;
        });

        let field_ty = &field.ty;
        let from_row_value = if type_info.nullable {
            quote! {
                row.try_get_typed::<#field_ty>(#column_name)?.flatten()
            }
        } else {
            quote! {
                row.get_required_typed::<#field_ty>(#column_name)?
            }
        };

        from_row_fields.push(quote! {
            #field_ident: #from_row_value
        });

        for index in config.indexes {
            let index_name = index.name.unwrap_or_else(|| {
                generated_index_name(
                    if index.unique { "ux" } else { "ix" },
                    table.value().as_str(),
                    column_name.value().as_str(),
                    field_ident.span(),
                )
            });
            let unique = index.unique;

            indexes.push(quote! {
                ::sql_orm::core::IndexMetadata {
                    name: #index_name,
                    columns: {
                        const COLUMNS: &[::sql_orm::core::IndexColumnMetadata] =
                            &[::sql_orm::core::IndexColumnMetadata::asc(#column_name)];
                        COLUMNS
                    },
                    unique: #unique,
                }
            });
        }

        if let Some(foreign_key) = config.foreign_key {
            let foreign_key_has_explicit_name = foreign_key.name.is_some();
            let foreign_key_name = foreign_key.name.clone().unwrap_or_else(|| {
                generated_foreign_key_name(
                    table.value().as_str(),
                    column_name.value().as_str(),
                    foreign_key.generated_referenced_table_name.as_str(),
                    field_ident.span(),
                )
            });
            let referenced_schema = foreign_key.referenced_schema_tokens();
            let referenced_table = foreign_key.referenced_table_tokens();
            let referenced_column = foreign_key.referenced_column_tokens();
            let on_delete = config
                .on_delete
                .unwrap_or(ReferentialActionConfig::NoAction);

            if on_delete == ReferentialActionConfig::SetNull && !nullable {
                return Err(Error::new_spanned(
                    &field.ty,
                    "on_delete = \"set null\" requiere un campo nullable",
                ));
            }

            let on_delete = referential_action_tokens(on_delete);

            foreign_keys.push(quote! {
                ::sql_orm::core::ForeignKeyMetadata::new(
                    #foreign_key_name,
                    {
                        const COLUMNS: &[&'static str] = &[#column_name];
                        COLUMNS
                    },
                    #referenced_schema,
                    #referenced_table,
                    {
                        const REFERENCED_COLUMNS: &[&'static str] = &[#referenced_column];
                        REFERENCED_COLUMNS
                    },
                    #on_delete,
                    ::sql_orm::core::ReferentialAction::NoAction,
                )
            });

            let foreign_key_accessor = format_ident!("__sql_orm_fk_{}", field_ident);
            foreign_key_accessors.push(quote! {
                #[doc(hidden)]
                pub fn #foreign_key_accessor() -> &'static ::sql_orm::core::ForeignKeyMetadata {
                    <Self as ::sql_orm::core::Entity>::metadata()
                        .foreign_key(#foreign_key_name)
                        .expect("generated foreign key accessor must reference existing metadata")
                }
            });

            field_foreign_keys.insert(
                field_ident.to_string(),
                FieldForeignKeyInfo {
                    name: foreign_key_name,
                    local_column: column_name.clone(),
                    referenced_column,
                    field_span: field_ident.span(),
                    has_explicit_name: foreign_key_has_explicit_name,
                    structured_target: foreign_key.structured_target_key(),
                },
            );
        }
    }

    validate_repeated_structured_foreign_keys(&field_foreign_keys)?;

    let navigation_metadata = navigations
        .iter()
        .map(|navigation| {
            let rust_field = &navigation.rust_field;
            let kind_tokens = &navigation.kind_tokens;
            let target = &navigation.target;
            let target_rust_name = &navigation.target_rust_name;
            let target_schema = quote! { #target::__SQL_ORM_ENTITY_SCHEMA };
            let target_table = quote! { #target::__SQL_ORM_ENTITY_TABLE };

            match navigation.kind {
                NavigationKindConfig::BelongsTo => {
                    let foreign_key = field_foreign_keys
                        .get(&navigation.foreign_key_field_name)
                        .ok_or_else(|| {
                            Error::new(
                                navigation.foreign_key_field.span(),
                                "belongs_to requiere foreign_key = campo_con_foreign_key existente",
                            )
                        })?;
                    let local_column = &foreign_key.local_column;
                    let referenced_column = &foreign_key.referenced_column;
                    let foreign_key_name = &foreign_key.name;
                    let navigation_target = path_key(&navigation.target);

                    let foreign_key_target = foreign_key.structured_target.as_ref().ok_or_else(|| {
                        Error::new(
                            navigation.foreign_key_field.span(),
                            "belongs_to requiere que foreign_key apunte a una foreign key estructurada: #[orm(foreign_key(entity = Target, column = id))]",
                        )
                    })?;

                    if foreign_key_target != &navigation_target {
                        return Err(Error::new(
                            navigation.foreign_key_field.span(),
                            "belongs_to requiere que el target coincida con la entidad declarada en foreign_key(entity = ...)",
                        ));
                    }

                    Ok(quote! {
                        ::sql_orm::core::NavigationMetadata::new(
                            #rust_field,
                            #kind_tokens,
                            #target_rust_name,
                            #target_schema,
                            #target_table,
                            {
                                const LOCAL_COLUMNS: &[&'static str] = &[#local_column];
                                LOCAL_COLUMNS
                            },
                            {
                                const TARGET_COLUMNS: &[&'static str] = &[#referenced_column];
                                TARGET_COLUMNS
                            },
                            Some(#foreign_key_name),
                        )
                    })
                }
                NavigationKindConfig::HasOne | NavigationKindConfig::HasMany => {
                    let foreign_key_field = &navigation.foreign_key_field;
                    let foreign_key_accessor =
                        format_ident!("__sql_orm_fk_{}", foreign_key_field);
                    Ok(quote! {
                        {
                            let foreign_key = #target::#foreign_key_accessor();
                            assert!(
                                foreign_key.references_table(#schema, #table),
                                "has_one/has_many requiere que foreign_key apunte a la entidad local",
                            );

                            ::sql_orm::core::NavigationMetadata::new(
                            #rust_field,
                            #kind_tokens,
                            #target_rust_name,
                            #target_schema,
                            #target_table,
                            {
                                foreign_key.referenced_columns
                            },
                            {
                                foreign_key.columns
                            },
                            Some(foreign_key.name),
                            )
                        }
                    })
                }
            }
        })
        .collect::<Result<Vec<_>>>()?;

    for index in entity_indexes {
        let resolved_columns = index
            .columns
            .iter()
            .map(|column| {
                field_columns
                    .get(&column.to_string())
                    .cloned()
                    .ok_or_else(|| {
                        Error::new_spanned(
                            column,
                            "index compuesto referencia un campo inexistente en la entidad",
                        )
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        let generated_suffix = resolved_columns
            .iter()
            .map(LitStr::value)
            .collect::<Vec<_>>()
            .join("_");
        let index_name = index.name.unwrap_or_else(|| {
            generated_index_name(
                if index.unique { "ux" } else { "ix" },
                table.value().as_str(),
                generated_suffix.as_str(),
                index.columns[0].span(),
            )
        });
        let unique = index.unique;

        indexes.push(quote! {
            ::sql_orm::core::IndexMetadata {
                name: #index_name,
                columns: {
                    const COLUMNS: &[::sql_orm::core::IndexColumnMetadata] =
                        &[#(::sql_orm::core::IndexColumnMetadata::asc(#resolved_columns)),*];
                    COLUMNS
                },
                unique: #unique,
            }
        });
    }

    if primary_key_columns.is_empty() {
        return Err(Error::new_spanned(
            ident,
            "Entity requiere al menos una primary key",
        ));
    }

    let metadata_ident = Ident::new(
        &format!("__SQL_ORM_ENTITY_METADATA_FOR_{}", ident),
        Span::call_site(),
    );
    let primary_key_value_impl = if primary_key_columns.len() == 1 {
        primary_key_value_expr.expect("single primary key must produce key extraction")
    } else {
        quote! {
            Err(::sql_orm::core::OrmError::compile(
                "ActiveRecord currently supports delete only for entities with a single primary key column",
            ))
        }
    };
    let persist_mode_impl = if primary_key_columns.len() == 1 {
        persist_mode_expr.expect("single primary key must produce save strategy")
    } else {
        quote! {
            Err(::sql_orm::core::OrmError::compile(
                "ActiveRecord currently supports save only for entities with a single primary key column",
            ))
        }
    };
    let entity_concurrency_token_impl = entity_concurrency_token.unwrap_or_else(|| {
        quote! {
            Ok(None)
        }
    });
    let audit_collision_checks = entity_audit
        .as_ref()
        .map(|audit| {
            entity_column_names
                .iter()
                .map(|column_name| {
                    quote! {
                        const _: () = assert!(
                            !::sql_orm::core::column_name_exists(
                                <#audit as ::sql_orm::core::EntityPolicy>::COLUMN_NAMES,
                                #column_name,
                            ),
                            concat!(
                                "audit policy column `",
                                #column_name,
                                "` collides with an entity column; rename the entity field with #[orm(column = \"...\")] or the AuditFields field with #[orm(column = \"...\")]",
                            ),
                        );
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let soft_delete_collision_checks = entity_soft_delete
        .as_ref()
        .map(|soft_delete| {
            entity_column_names
                .iter()
                .map(|column_name| {
                    quote! {
                        const _: () = assert!(
                            !::sql_orm::core::column_name_exists(
                                <#soft_delete as ::sql_orm::core::EntityPolicy>::COLUMN_NAMES,
                                #column_name,
                            ),
                            concat!(
                                "soft_delete policy column `",
                                #column_name,
                                "` collides with an entity column; rename the entity field with #[orm(column = \"...\")] or the soft delete policy field with #[orm(column = \"...\")]",
                            ),
                        );
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let tenant_collision_checks = entity_tenant
        .as_ref()
        .map(|tenant| {
            entity_column_names
                .iter()
                .map(|column_name| {
                    quote! {
                        const _: () = assert!(
                            !::sql_orm::core::column_name_exists(
                                <#tenant as ::sql_orm::core::EntityPolicy>::COLUMN_NAMES,
                                #column_name,
                            ),
                            concat!(
                                "tenant policy column `",
                                #column_name,
                                "` collides with an entity column; rename the entity field with #[orm(column = \"...\")] or the TenantContext field with #[orm(column = \"...\")]",
                            ),
                        );
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let audit_soft_delete_collision_checks = match (&entity_audit, &entity_soft_delete) {
        (Some(audit), Some(soft_delete)) => {
            quote! {
                const _: () = {
                    let soft_delete_columns =
                        <#soft_delete as ::sql_orm::core::EntityPolicy>::COLUMN_NAMES;
                    let audit_columns = <#audit as ::sql_orm::core::EntityPolicy>::COLUMN_NAMES;
                    let mut index = 0;
                    while index < soft_delete_columns.len() {
                        assert!(
                            !::sql_orm::core::column_name_exists(
                                audit_columns,
                                soft_delete_columns[index],
                            ),
                            "soft_delete policy columns collide with audit policy columns; rename one of the generated columns explicitly",
                        );
                        index += 1;
                    }
                };
            }
        }
        _ => quote! {},
    };
    let tenant_policy_collision_checks = {
        let mut checks = Vec::new();

        if let (Some(audit), Some(tenant)) = (&entity_audit, &entity_tenant) {
            checks.push(quote! {
                const _: () = {
                    let tenant_columns =
                        <#tenant as ::sql_orm::core::EntityPolicy>::COLUMN_NAMES;
                    let audit_columns = <#audit as ::sql_orm::core::EntityPolicy>::COLUMN_NAMES;
                    let mut index = 0;
                    while index < tenant_columns.len() {
                        assert!(
                            !::sql_orm::core::column_name_exists(
                                audit_columns,
                                tenant_columns[index],
                            ),
                            "tenant policy columns collide with audit policy columns; rename one of the generated columns explicitly",
                        );
                        index += 1;
                    }
                };
            });
        }

        if let (Some(soft_delete), Some(tenant)) = (&entity_soft_delete, &entity_tenant) {
            checks.push(quote! {
                const _: () = {
                    let tenant_columns =
                        <#tenant as ::sql_orm::core::EntityPolicy>::COLUMN_NAMES;
                    let soft_delete_columns =
                        <#soft_delete as ::sql_orm::core::EntityPolicy>::COLUMN_NAMES;
                    let mut index = 0;
                    while index < tenant_columns.len() {
                        assert!(
                            !::sql_orm::core::column_name_exists(
                                soft_delete_columns,
                                tenant_columns[index],
                            ),
                            "tenant policy columns collide with soft_delete policy columns; rename one of the generated columns explicitly",
                        );
                        index += 1;
                    }
                };
            });
        }

        checks
    };
    let tenant_context_bound_check = entity_tenant.as_ref().map(|tenant| {
        quote! {
            const _: &'static str = <#tenant as ::sql_orm::TenantContext>::COLUMN_NAME;
        }
    });
    let primary_key_metadata = quote! {
        ::sql_orm::core::PrimaryKeyMetadata::new(
            None,
            {
                const COLUMNS: &[&'static str] = &[#(#primary_key_columns),*];
                COLUMNS
            },
        )
    };
    let indexes_metadata = quote! {
        {
            const INDEXES: &[::sql_orm::core::IndexMetadata] = &[#(#indexes),*];
            INDEXES
        }
    };
    let foreign_keys_metadata = quote! {
        {
            const FOREIGN_KEYS: &[::sql_orm::core::ForeignKeyMetadata] = &[#(#foreign_keys),*];
            FOREIGN_KEYS
        }
    };
    let navigations_metadata = quote! {
        {
            const NAVIGATIONS: &[::sql_orm::core::NavigationMetadata] =
                &[#(#navigation_metadata),*];
            NAVIGATIONS
        }
    };

    let has_inverse_navigation_metadata = navigations.iter().any(|navigation| {
        matches!(
            navigation.kind,
            NavigationKindConfig::HasOne | NavigationKindConfig::HasMany
        )
    });
    let has_generated_policies =
        entity_audit.is_some() || entity_soft_delete.is_some() || entity_tenant.is_some();
    let audit_columns_extend = entity_audit.as_ref().map(|audit| {
        quote! {
            columns.extend_from_slice(
                <#audit as ::sql_orm::core::EntityPolicy>::columns()
            );
        }
    });
    let soft_delete_columns_extend = entity_soft_delete.as_ref().map(|soft_delete| {
        quote! {
            columns.extend_from_slice(
                <#soft_delete as ::sql_orm::core::EntityPolicy>::columns()
            );
        }
    });
    let tenant_columns_extend = entity_tenant.as_ref().map(|tenant| {
        quote! {
            columns.extend_from_slice(
                <#tenant as ::sql_orm::core::EntityPolicy>::columns()
            );
        }
    });
    let audit_contract_impl = entity_audit.as_ref().map_or_else(
        || {
            quote! {
                impl ::sql_orm::AuditEntity for #ident {
                    fn audit_policy() -> Option<::sql_orm::core::EntityPolicyMetadata> {
                        None
                    }
                }
            }
        },
        |audit| {
            quote! {
                impl ::sql_orm::AuditEntity for #ident {
                    fn audit_policy() -> Option<::sql_orm::core::EntityPolicyMetadata> {
                        Some(<#audit as ::sql_orm::core::EntityPolicy>::metadata())
                    }
                }
            }
        },
    );
    let soft_delete_contract_impl = entity_soft_delete.as_ref().map_or_else(
        || {
            quote! {
                impl ::sql_orm::SoftDeleteEntity for #ident {
                    fn soft_delete_policy() -> Option<::sql_orm::core::EntityPolicyMetadata> {
                        None
                    }
                }
            }
        },
        |soft_delete| {
            quote! {
                impl ::sql_orm::SoftDeleteEntity for #ident {
                    fn soft_delete_policy() -> Option<::sql_orm::core::EntityPolicyMetadata> {
                        Some(<#soft_delete as ::sql_orm::core::EntityPolicy>::metadata())
                    }
                }
            }
        },
    );
    let tenant_contract_impl = entity_tenant.as_ref().map_or_else(
        || {
            quote! {
                impl ::sql_orm::TenantScopedEntity for #ident {
                    fn tenant_policy() -> Option<::sql_orm::core::EntityPolicyMetadata> {
                        None
                    }
                }
            }
        },
        |tenant| {
            quote! {
                impl ::sql_orm::TenantScopedEntity for #ident {
                    fn tenant_policy() -> Option<::sql_orm::core::EntityPolicyMetadata> {
                        Some(<#tenant as ::sql_orm::core::EntityPolicy>::metadata())
                    }
                }
            }
        },
    );
    let include_navigation_impls = include_navigation_impls(&ident, &navigations)?;
    let include_collection_impls = include_collection_impls(&ident, &navigations)?;

    let (metadata_static, metadata_expr) = if has_generated_policies
        || has_inverse_navigation_metadata
    {
        (
            quote! {
                static #metadata_ident: ::std::sync::OnceLock<::sql_orm::core::EntityMetadata> =
                    ::std::sync::OnceLock::new();
            },
            quote! {
                #metadata_ident.get_or_init(|| {
                    let mut columns = ::std::vec::Vec::new();
                    columns.extend_from_slice(&[#(#columns),*]);
                    #audit_columns_extend
                    #soft_delete_columns_extend
                    #tenant_columns_extend
                    let columns: &'static [::sql_orm::core::ColumnMetadata] =
                        ::std::boxed::Box::leak(columns.into_boxed_slice());
                    let navigations: &'static [::sql_orm::core::NavigationMetadata] =
                        ::std::boxed::Box::leak(::std::vec![#(#navigation_metadata),*].into_boxed_slice());

                    ::sql_orm::core::EntityMetadata {
                        rust_name: #rust_name,
                        schema: #schema,
                        table: #table,
                        renamed_from: #renamed_from,
                        columns,
                        primary_key: #primary_key_metadata,
                        indexes: #indexes_metadata,
                        foreign_keys: #foreign_keys_metadata,
                        navigations,
                    }
                })
            },
        )
    } else {
        (
            quote! {
                static #metadata_ident: ::sql_orm::core::EntityMetadata =
                    ::sql_orm::core::EntityMetadata {
                        rust_name: #rust_name,
                        schema: #schema,
                        table: #table,
                        renamed_from: #renamed_from,
                        columns: &[#(#columns),*],
                        primary_key: #primary_key_metadata,
                        indexes: #indexes_metadata,
                        foreign_keys: #foreign_keys_metadata,
                        navigations: #navigations_metadata,
                    };
            },
            quote! {
                &#metadata_ident
            },
        )
    };

    Ok(quote! {
        #(#audit_collision_checks)*
        #(#soft_delete_collision_checks)*
        #(#tenant_collision_checks)*
        #audit_soft_delete_collision_checks
        #(#tenant_policy_collision_checks)*
        #tenant_context_bound_check

        #metadata_static

        #[allow(non_upper_case_globals)]
        impl #ident {
            #[doc(hidden)]
            pub const __SQL_ORM_ENTITY_SCHEMA: &'static str = #schema;

            #[doc(hidden)]
            pub const __SQL_ORM_ENTITY_TABLE: &'static str = #table;

            #(#column_symbols)*
            #(#foreign_key_accessors)*
        }

        impl ::sql_orm::core::Entity for #ident {
            fn metadata() -> &'static ::sql_orm::core::EntityMetadata {
                #metadata_expr
            }
        }

        impl ::sql_orm::core::FromRow for #ident {
            fn from_row<R: ::sql_orm::core::Row>(row: &R) -> Result<Self, ::sql_orm::core::OrmError> {
                Ok(Self {
                    #(#from_row_fields),*
                })
            }
        }

        impl ::sql_orm::EntityPrimaryKey for #ident {
            fn primary_key_value(&self) -> Result<::sql_orm::core::SqlValue, ::sql_orm::core::OrmError> {
                #primary_key_value_impl
            }
        }

        impl ::sql_orm::EntityPersist for #ident {
            fn persist_mode(&self) -> Result<::sql_orm::EntityPersistMode, ::sql_orm::core::OrmError> {
                #persist_mode_impl
            }

            fn insert_values(&self) -> ::std::vec::Vec<::sql_orm::core::ColumnValue> {
                let mut values = ::std::vec::Vec::new();
                #(#insert_values)*
                values
            }

            fn update_changes(&self) -> ::std::vec::Vec<::sql_orm::core::ColumnValue> {
                let mut changes = ::std::vec::Vec::new();
                #(#update_changes)*
                changes
            }

            fn concurrency_token(&self) -> Result<::core::option::Option<::sql_orm::core::SqlValue>, ::sql_orm::core::OrmError> {
                #entity_concurrency_token_impl
            }

            fn sync_persisted(&mut self, persisted: Self) {
                #(#sync_fields)*
            }
        }

        #audit_contract_impl
        #soft_delete_contract_impl
        #tenant_contract_impl
        #(#include_navigation_impls)*
        #(#include_collection_impls)*
    })
}

fn derive_db_context_impl(input: DeriveInput) -> Result<TokenStream2> {
    let context_ident = input.ident.clone();
    let ident = input.ident;
    let fields = extract_named_fields(&ident, input.data, "DbContext")?;
    let shared_connection_field = fields
        .first()
        .and_then(|field| field.ident.as_ref())
        .ok_or_else(|| {
            Error::new_spanned(
                &ident,
                "DbContext requiere al menos un campo DbSet<Entidad>",
            )
        })?;

    let mut seen_entities = std::collections::HashSet::new();
    for field in &fields {
        let entity_type = dbset_entity_type(&field.ty).ok_or_else(|| {
            Error::new_spanned(
                &field.ty,
                "DbContext requiere campos con tipo DbSet<Entidad>",
            )
        })?;
        let entity_key = quote! { #entity_type }.to_string();
        if !seen_entities.insert(entity_key) {
            return Err(Error::new_spanned(
                &field.ty,
                "DbContext no soporta múltiples DbSet para la misma entidad",
            ));
        }
    }

    let initializers = fields
        .iter()
        .map(|field| {
            let field_ident = field
                .ident
                .as_ref()
                .ok_or_else(|| Error::new_spanned(field, "DbContext requiere campos nombrados"))?;
            let entity_type = dbset_entity_type(&field.ty).ok_or_else(|| {
                Error::new_spanned(
                    &field.ty,
                    "DbContext requiere campos con tipo DbSet<Entidad>",
                )
            })?;

            Ok(quote! {
                #field_ident: ::sql_orm::DbSet::<#entity_type>::with_tracking_registry(
                    connection.clone(),
                    ::std::sync::Arc::clone(&tracking_registry)
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let dbset_access_impls = fields
        .iter()
        .map(|field| {
            let field_ident = field
                .ident
                .as_ref()
                .ok_or_else(|| Error::new_spanned(field, "DbContext requiere campos nombrados"))?;
            let entity_type = dbset_entity_type(&field.ty).ok_or_else(|| {
                Error::new_spanned(
                    &field.ty,
                    "DbContext requiere campos con tipo DbSet<Entidad>",
                )
            })?;

            Ok(quote! {
                impl ::sql_orm::DbContextEntitySet<#entity_type> for #context_ident {
                    fn db_set(&self) -> &::sql_orm::DbSet<#entity_type> {
                        &self.#field_ident
                    }
                }
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let save_plan_entity_metadata = fields
        .iter()
        .map(|field| {
            let entity_type = dbset_entity_type(&field.ty).ok_or_else(|| {
                Error::new_spanned(
                    &field.ty,
                    "DbContext requiere campos con tipo DbSet<Entidad>",
                )
            })?;

            Ok(quote! {
                <#entity_type as ::sql_orm::core::Entity>::metadata()
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let save_added_steps = fields
        .iter()
        .enumerate()
        .map(|(field_index, field)| {
            let field_ident = field
                .ident
                .as_ref()
                .ok_or_else(|| Error::new_spanned(field, "DbContext requiere campos nombrados"))?;
            Ok(quote! {
                #field_index => {
                    saved += self.#field_ident.save_tracked_added().await?;
                }
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let save_modified_steps = fields
        .iter()
        .enumerate()
        .map(|(field_index, field)| {
            let field_ident = field
                .ident
                .as_ref()
                .ok_or_else(|| Error::new_spanned(field, "DbContext requiere campos nombrados"))?;
            Ok(quote! {
                #field_index => {
                    saved += self.#field_ident.save_tracked_modified().await?;
                }
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let save_deleted_steps = fields
        .iter()
        .enumerate()
        .map(|(field_index, field)| {
            let field_ident = field
                .ident
                .as_ref()
                .ok_or_else(|| Error::new_spanned(field, "DbContext requiere campos nombrados"))?;
            Ok(quote! {
                #field_index => {
                    saved += self.#field_ident.save_tracked_deleted().await?;
                }
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let save_changes_bounds = fields
        .iter()
        .map(|field| {
            let entity_type = dbset_entity_type(&field.ty).ok_or_else(|| {
                Error::new_spanned(
                    &field.ty,
                    "DbContext requiere campos con tipo DbSet<Entidad>",
                )
            })?;

            Ok(quote! {
                #entity_type: ::core::clone::Clone
                    + ::sql_orm::EntityPersist
                    + ::sql_orm::EntityPrimaryKey
                    + ::sql_orm::SoftDeleteEntity
                    + ::sql_orm::TenantScopedEntity
                    + ::sql_orm::core::FromRow
                    + ::core::marker::Send
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let migration_entity_metadata_static = Ident::new(
        &format!("__{}_MIGRATION_ENTITY_METADATA", ident),
        Span::call_site(),
    );
    let migration_entity_metadata = fields
        .iter()
        .map(|field| {
            let entity_type = dbset_entity_type(&field.ty).ok_or_else(|| {
                Error::new_spanned(
                    &field.ty,
                    "DbContext requiere campos con tipo DbSet<Entidad>",
                )
            })?;

            Ok(quote! {
                <#entity_type as ::sql_orm::core::Entity>::metadata()
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(quote! {
        impl #ident {
            fn __from_shared_parts(
                connection: ::sql_orm::SharedConnection,
                tracking_registry: ::sql_orm::TrackingRegistryHandle,
            ) -> Self {
                Self {
                    #(#initializers),*
                }
            }
        }

        impl ::sql_orm::DbContext for #ident {
            fn from_shared_connection(connection: ::sql_orm::SharedConnection) -> Self {
                let tracking_registry =
                    ::std::sync::Arc::new(::sql_orm::TrackingRegistry::default());
                Self::__from_shared_parts(connection, tracking_registry)
            }

            fn shared_connection(&self) -> ::sql_orm::SharedConnection {
                self.#shared_connection_field.shared_connection()
            }

            fn tracking_registry(&self) -> ::sql_orm::TrackingRegistryHandle {
                self.#shared_connection_field.tracking_registry()
            }
        }

        impl #ident {
            pub fn from_shared_connection(connection: ::sql_orm::SharedConnection) -> Self {
                <Self as ::sql_orm::DbContext>::from_shared_connection(connection)
            }

            pub fn with_audit_provider(
                &self,
                provider: ::std::sync::Arc<dyn ::sql_orm::AuditProvider>,
            ) -> Self {
                let tracking_registry =
                    <Self as ::sql_orm::DbContext>::tracking_registry(self);
                let connection =
                    <Self as ::sql_orm::DbContext>::shared_connection(self)
                        .with_audit_provider(provider);
                Self::__from_shared_parts(connection, tracking_registry)
            }

            pub fn with_audit_request_values(
                &self,
                request_values: ::sql_orm::AuditRequestValues,
            ) -> Self {
                let tracking_registry =
                    <Self as ::sql_orm::DbContext>::tracking_registry(self);
                let connection =
                    <Self as ::sql_orm::DbContext>::shared_connection(self)
                        .with_audit_request_values(request_values);
                Self::__from_shared_parts(connection, tracking_registry)
            }

            pub fn with_audit_values<V>(&self, values: V) -> Self
            where
                V: ::sql_orm::AuditValues,
            {
                let tracking_registry =
                    <Self as ::sql_orm::DbContext>::tracking_registry(self);
                let connection =
                    <Self as ::sql_orm::DbContext>::shared_connection(self)
                        .with_audit_values(values);
                Self::__from_shared_parts(connection, tracking_registry)
            }

            pub fn clear_audit_request_values(&self) -> Self {
                let tracking_registry =
                    <Self as ::sql_orm::DbContext>::tracking_registry(self);
                let connection =
                    <Self as ::sql_orm::DbContext>::shared_connection(self)
                        .clear_audit_request_values();
                Self::__from_shared_parts(connection, tracking_registry)
            }

            pub fn with_soft_delete_provider(
                &self,
                provider: ::std::sync::Arc<dyn ::sql_orm::SoftDeleteProvider>,
            ) -> Self {
                let tracking_registry =
                    <Self as ::sql_orm::DbContext>::tracking_registry(self);
                let connection =
                    <Self as ::sql_orm::DbContext>::shared_connection(self)
                        .with_soft_delete_provider(provider);
                Self::__from_shared_parts(connection, tracking_registry)
            }

            pub fn with_soft_delete_request_values(
                &self,
                request_values: ::sql_orm::SoftDeleteRequestValues,
            ) -> Self {
                let tracking_registry =
                    <Self as ::sql_orm::DbContext>::tracking_registry(self);
                let connection =
                    <Self as ::sql_orm::DbContext>::shared_connection(self)
                        .with_soft_delete_request_values(request_values);
                Self::__from_shared_parts(connection, tracking_registry)
            }

            pub fn with_soft_delete_values<V>(&self, values: V) -> Self
            where
                V: ::sql_orm::SoftDeleteValues,
            {
                let tracking_registry =
                    <Self as ::sql_orm::DbContext>::tracking_registry(self);
                let connection =
                    <Self as ::sql_orm::DbContext>::shared_connection(self)
                        .with_soft_delete_values(values);
                Self::__from_shared_parts(connection, tracking_registry)
            }

            pub fn clear_soft_delete_request_values(&self) -> Self {
                let tracking_registry =
                    <Self as ::sql_orm::DbContext>::tracking_registry(self);
                let connection =
                    <Self as ::sql_orm::DbContext>::shared_connection(self)
                        .clear_soft_delete_request_values();
                Self::__from_shared_parts(connection, tracking_registry)
            }

            pub fn with_tenant<T>(&self, tenant: T) -> Self
            where
                T: ::sql_orm::TenantContext,
            {
                let tracking_registry =
                    <Self as ::sql_orm::DbContext>::tracking_registry(self);
                let connection =
                    <Self as ::sql_orm::DbContext>::shared_connection(self)
                        .with_tenant(tenant);
                Self::__from_shared_parts(connection, tracking_registry)
            }

            pub fn clear_tenant(&self) -> Self {
                let tracking_registry =
                    <Self as ::sql_orm::DbContext>::tracking_registry(self);
                let connection =
                    <Self as ::sql_orm::DbContext>::shared_connection(self)
                        .clear_tenant();
                Self::__from_shared_parts(connection, tracking_registry)
            }

            pub fn from_connection(
                connection: ::sql_orm::tiberius::MssqlConnection<
                    ::sql_orm::tiberius::TokioConnectionStream
                >,
            ) -> Self {
                <Self as ::sql_orm::DbContext>::from_shared_connection(
                    ::sql_orm::SharedConnection::from_connection(connection)
                )
            }

            #[cfg(feature = "pool-bb8")]
            pub fn from_pool(pool: ::sql_orm::MssqlPool) -> Self {
                <Self as ::sql_orm::DbContext>::from_shared_connection(
                    ::sql_orm::SharedConnection::from_pool(pool)
                )
            }

            pub async fn connect(connection_string: &str) -> Result<Self, ::sql_orm::core::OrmError> {
                let connection = ::sql_orm::tiberius::MssqlConnection::connect(connection_string)
                    .await?;
                Ok(Self::from_connection(connection))
            }

            pub async fn connect_with_options(
                connection_string: &str,
                options: ::sql_orm::MssqlOperationalOptions,
            ) -> Result<Self, ::sql_orm::core::OrmError> {
                let config = ::sql_orm::MssqlConnectionConfig::from_connection_string_with_options(
                    connection_string,
                    options,
                )?;
                Self::connect_with_config(config).await
            }

            pub async fn connect_with_config(
                config: ::sql_orm::MssqlConnectionConfig,
            ) -> Result<Self, ::sql_orm::core::OrmError> {
                let connection =
                    ::sql_orm::tiberius::MssqlConnection::connect_with_config(config).await?;
                Ok(Self::from_connection(connection))
            }

            pub async fn transaction<F, Fut, T>(&self, operation: F) -> Result<T, ::sql_orm::core::OrmError>
            where
                F: FnOnce(Self) -> Fut + Send,
                Fut: ::core::future::Future<Output = Result<T, ::sql_orm::core::OrmError>> + Send,
                T: Send,
            {
                let shared_connection =
                    <Self as ::sql_orm::DbContext>::shared_connection(self);
                let tracking_registry =
                    <Self as ::sql_orm::DbContext>::tracking_registry(self);

                shared_connection.run_transaction(|transaction_connection| async move {
                    let transaction_context =
                        Self::__from_shared_parts(transaction_connection, tracking_registry);
                    operation(transaction_context).await
                }).await
            }

            pub async fn health_check(&self) -> Result<(), ::sql_orm::core::OrmError> {
                <Self as ::sql_orm::DbContext>::health_check(self).await
            }

            /// Clears every experimental tracking entry currently registered
            /// on this context.
            ///
            /// This does not execute SQL and does not reset values already
            /// held by `Tracked<T>` wrappers. It only detaches the current
            /// unit-of-work entries so later `save_changes()` calls ignore
            /// them.
            pub fn clear_tracker(&self) {
                <Self as ::sql_orm::DbContext>::clear_tracker(self)
            }

            async fn __sql_orm_save_changes_without_transaction(&self) -> Result<usize, ::sql_orm::core::OrmError>
            where
                #(#save_changes_bounds,)*
            {
                let mut saved = 0usize;
                let save_plan = ::sql_orm::save_changes_operation_plan(&[
                    #(#save_plan_entity_metadata),*
                ])?;

                for entity_index in save_plan.added_order() {
                    match *entity_index {
                        #(#save_added_steps)*
                        _ => {}
                    }
                }

                for entity_index in save_plan.modified_order() {
                    match *entity_index {
                        #(#save_modified_steps)*
                        _ => {}
                    }
                }

                for entity_index in save_plan.deleted_order() {
                    match *entity_index {
                        #(#save_deleted_steps)*
                        _ => {}
                    }
                }

                Ok(saved)
            }

            /// Persists currently registered experimental tracking entries.
            ///
            /// This method processes live `Tracked<T>` wrappers registered by
            /// this context in three deterministic phases: `Added`,
            /// `Modified`, then `Deleted`. For simple foreign keys declared
            /// between entities in the context, inserts and updates run
            /// principals before dependents and deletes run the reverse order.
            ///
            /// The implementation reuses the normal `DbSet` insert, update
            /// and delete paths, so tenant, audit, soft-delete and rowversion
            /// behavior remains centralized in the public persistence layer.
            /// It opens an internal transaction when no transaction is active
            /// and reuses an outer `db.transaction(...)` when one is already
            /// active.
            ///
            /// Tracking remains experimental in this release slice: dropping a
            /// wrapper detaches it, relationship graph mutations are not
            /// persisted, and composite primary keys are rejected before SQL
            /// for pending tracked operations.
            pub async fn save_changes(&self) -> Result<usize, ::sql_orm::core::OrmError>
            where
                #(#save_changes_bounds,)*
            {
                let shared_connection =
                    <Self as ::sql_orm::DbContext>::shared_connection(self);

                if shared_connection.is_transaction_active() {
                    self.__sql_orm_save_changes_without_transaction().await
                } else {
                    let tracking_registry =
                        <Self as ::sql_orm::DbContext>::tracking_registry(self);
                    shared_connection.run_transaction(|transaction_connection| async move {
                        let transaction_context =
                            Self::__from_shared_parts(transaction_connection, tracking_registry);
                        transaction_context.__sql_orm_save_changes_without_transaction().await
                    }).await
                }
            }
        }

        impl ::sql_orm::MigrationModelSource for #ident {
            fn entity_metadata() -> &'static [&'static ::sql_orm::EntityMetadata] {
                static #migration_entity_metadata_static:
                    ::std::sync::OnceLock<
                        ::std::boxed::Box<[&'static ::sql_orm::EntityMetadata]>
                    > = ::std::sync::OnceLock::new();

                #migration_entity_metadata_static
                    .get_or_init(|| {
                        ::std::boxed::Box::new([#(#migration_entity_metadata),*])
                    })
                    .as_ref()
            }
        }

        #(#dbset_access_impls)*
    })
}

fn derive_insertable_impl(input: DeriveInput) -> Result<TokenStream2> {
    let ident = input.ident;
    let model_config = parse_persistence_model_config(&input.attrs, "Insertable")?;
    let entity = model_config
        .entity
        .as_ref()
        .expect("validated persistence model must include entity");
    let fields = extract_named_fields(&ident, input.data, "Insertable")?;

    let values = fields
        .iter()
        .map(|field| {
            let field_ident = field
                .ident
                .as_ref()
                .ok_or_else(|| Error::new_spanned(field, "Insertable requiere campos nombrados"))?;
            let field_config = parse_persistence_field_config(field, "Insertable")?;
            let field_ty = &field.ty;
            let column_name =
                persistence_column_name_expr(entity, field_ident, field_config.column.as_ref());

            Ok(quote! {
                ::sql_orm::core::ColumnValue::new(
                    #column_name,
                    <#field_ty as ::sql_orm::core::SqlTypeMapping>::to_sql_value(
                        ::core::clone::Clone::clone(&self.#field_ident)
                    ),
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(quote! {
        impl ::sql_orm::core::Insertable<#entity> for #ident {
            fn values(&self) -> ::std::vec::Vec<::sql_orm::core::ColumnValue> {
                ::std::vec![#(#values),*]
            }
        }
    })
}

fn derive_changeset_impl(input: DeriveInput) -> Result<TokenStream2> {
    let ident = input.ident;
    let model_config = parse_persistence_model_config(&input.attrs, "Changeset")?;
    let entity = model_config
        .entity
        .as_ref()
        .expect("validated persistence model must include entity");
    let fields = extract_named_fields(&ident, input.data, "Changeset")?;

    let changes = fields
        .iter()
        .map(|field| {
            let field_ident = field
                .ident
                .as_ref()
                .ok_or_else(|| Error::new_spanned(field, "Changeset requiere campos nombrados"))?;
            let field_config = parse_persistence_field_config(field, "Changeset")?;
            let inner_ty = option_inner_type(&field.ty).ok_or_else(|| {
                Error::new_spanned(
                    &field.ty,
                    "Changeset requiere Option<T> en cada campo para distinguir campos omitidos",
                )
            })?;
            let column_name =
                persistence_column_name_expr(entity, field_ident, field_config.column.as_ref());

            Ok(quote! {
                let column_name = #column_name;
                let column = <#entity as ::sql_orm::core::Entity>::metadata()
                    .column(column_name)
                    .expect("generated Changeset field must reference existing entity metadata");

                if let ::core::option::Option::Some(value) = &self.#field_ident {
                    if column.updatable {
                        changes.push(::sql_orm::core::ColumnValue::new(
                            column_name,
                            <#inner_ty as ::sql_orm::core::SqlTypeMapping>::to_sql_value(
                                ::core::clone::Clone::clone(value)
                            ),
                        ));
                    }
                }
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let concurrency_tokens = fields
        .iter()
        .map(|field| {
            let field_ident = field
                .ident
                .as_ref()
                .ok_or_else(|| Error::new_spanned(field, "Changeset requiere campos nombrados"))?;
            let field_config = parse_persistence_field_config(field, "Changeset")?;
            let inner_ty = option_inner_type(&field.ty).ok_or_else(|| {
                Error::new_spanned(
                    &field.ty,
                    "Changeset requiere Option<T> en cada campo para distinguir campos omitidos",
                )
            })?;
            let column_name =
                persistence_column_name_expr(entity, field_ident, field_config.column.as_ref());

            Ok(quote! {
                let column_name = #column_name;
                let column = <#entity as ::sql_orm::core::Entity>::metadata()
                    .column(column_name)
                    .expect("generated Changeset field must reference existing entity metadata");

                if column.rowversion {
                    if let ::core::option::Option::Some(value) = &self.#field_ident {
                        return Ok(Some(
                            <#inner_ty as ::sql_orm::core::SqlTypeMapping>::to_sql_value(
                                ::core::clone::Clone::clone(value)
                            )
                        ));
                    }
                }
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(quote! {
        impl ::sql_orm::core::Changeset<#entity> for #ident {
            fn changes(&self) -> ::std::vec::Vec<::sql_orm::core::ColumnValue> {
                let mut changes = ::std::vec::Vec::new();
                #(#changes)*
                changes
            }

            fn concurrency_token(&self) -> Result<::core::option::Option<::sql_orm::core::SqlValue>, ::sql_orm::core::OrmError> {
                #(#concurrency_tokens)*
                Ok(None)
            }
        }
    })
}

fn extract_named_fields(
    ident: &Ident,
    data: Data,
    derive_name: &str,
) -> Result<syn::punctuated::Punctuated<Field, syn::token::Comma>> {
    match data {
        Data::Struct(data) => match data.fields {
            Fields::Named(fields) => Ok(fields.named),
            _ => Err(Error::new_spanned(
                ident,
                format!("{derive_name} solo soporta structs con campos nombrados"),
            )),
        },
        _ => Err(Error::new_spanned(
            ident,
            format!("{derive_name} solo soporta structs"),
        )),
    }
}

fn has_explicit_primary_key(
    fields: &syn::punctuated::Punctuated<Field, syn::token::Comma>,
) -> Result<bool> {
    for field in fields {
        if parse_field_config(field)?.primary_key {
            return Ok(true);
        }
    }
    Ok(false)
}

fn parse_persistence_model_config(
    attrs: &[syn::Attribute],
    derive_name: &str,
) -> Result<PersistenceModelConfig> {
    let mut config = PersistenceModelConfig::default();

    for attr in attrs {
        if !attr.path().is_ident("orm") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("entity") {
                config.entity = Some(meta.value()?.parse()?);
            } else {
                return Err(meta.error(format!(
                    "atributo orm no soportado a nivel de {derive_name}"
                )));
            }

            Ok(())
        })?;
    }

    let Some(entity) = config.entity else {
        return Err(Error::new(
            Span::call_site(),
            format!("{derive_name} requiere #[orm(entity = MiEntidad)]"),
        ));
    };

    Ok(PersistenceModelConfig {
        entity: Some(entity),
    })
}

fn parse_entity_config(attrs: &[syn::Attribute]) -> Result<EntityConfig> {
    let mut config = EntityConfig::default();

    for attr in attrs {
        if !attr.path().is_ident("orm") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("table") {
                config.table = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("schema") {
                config.schema = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("renamed_from") {
                config.renamed_from = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("audit") {
                if config.audit.is_some() {
                    return Err(meta.error(
                        "Entity solo soporta una policy audit; multiples policies que generen columnas solapadas deben rechazarse explicitamente",
                    ));
                }
                config.audit = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("soft_delete") {
                if config.soft_delete.is_some() {
                    return Err(meta.error(
                        "Entity solo soporta una policy soft_delete; multiples policies que generen columnas solapadas deben rechazarse explicitamente",
                    ));
                }
                config.soft_delete = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("tenant") {
                if config.tenant.is_some() {
                    return Err(meta.error(
                        "Entity solo soporta una policy tenant; multiples tenants deben rechazarse explicitamente",
                    ));
                }
                config.tenant = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("index") {
                config.indexes.push(parse_entity_index_config(meta)?);
            } else {
                return Err(meta.error("atributo orm no soportado a nivel de entidad"));
            }

            Ok(())
        })?;
    }

    Ok(config)
}

fn parse_entity_index_config(meta: syn::meta::ParseNestedMeta<'_>) -> Result<EntityIndexConfig> {
    let mut index = EntityIndexConfig::default();

    meta.parse_nested_meta(|nested| {
        if nested.path.is_ident("name") {
            index.name = Some(parse_lit_str(nested.value()?.parse()?)?);
        } else if nested.path.is_ident("unique") {
            index.unique = true;
        } else if nested.path.is_ident("columns") {
            let content;
            syn::parenthesized!(content in nested.input);
            let columns = Punctuated::<Ident, Token![,]>::parse_terminated(&content)?;
            index.columns.extend(columns);
        } else {
            return Err(nested.error("index de entidad solo soporta name, unique y columns(...)"));
        }

        Ok(())
    })?;

    if index.columns.is_empty() {
        return Err(meta.error("index a nivel de entidad requiere columns(campo1, campo2, ...)"));
    }

    Ok(index)
}

fn parse_persistence_field_config(
    field: &Field,
    derive_name: &str,
) -> Result<PersistenceFieldConfig> {
    let mut config = PersistenceFieldConfig::default();

    for attr in &field.attrs {
        if !attr.path().is_ident("orm") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("column") {
                config.column = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else {
                return Err(meta.error(format!(
                    "atributo orm no soportado en campos de {derive_name}"
                )));
            }

            Ok(())
        })?;
    }

    Ok(config)
}

fn parse_field_config(field: &Field) -> Result<FieldConfig> {
    let mut config = FieldConfig::default();

    for attr in &field.attrs {
        if !attr.path().is_ident("orm") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("column") {
                config.column = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("primary_key") {
                config.primary_key = true;
            } else if meta.path.is_ident("identity") {
                config.identity = true;
                if !meta.input.is_empty() {
                    meta.parse_nested_meta(|nested| {
                        if nested.path.is_ident("seed") {
                            config.identity_seed = Some(parse_i64_expr(nested.value()?.parse()?)?);
                        } else if nested.path.is_ident("increment") {
                            config.identity_increment =
                                Some(parse_i64_expr(nested.value()?.parse()?)?);
                        } else {
                            return Err(nested.error("identity solo soporta seed e increment"));
                        }

                        Ok(())
                    })?;
                }
            } else if meta.path.is_ident("length") {
                config.length = Some(parse_u32_expr(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("nullable") {
                config.nullable = true;
            } else if meta.path.is_ident("default_sql") {
                return Err(meta.error(
                    "default_sql es un fragmento SQL unsafe; usa unsafe_default_sql = \"...\" para reconocer explícitamente el riesgo",
                ));
            } else if meta.path.is_ident("unsafe_default_sql") {
                config.default_sql = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("renamed_from") {
                config.renamed_from = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("index") {
                let mut index = IndexConfig::default();
                meta.parse_nested_meta(|nested| {
                    if nested.path.is_ident("name") {
                        index.name = Some(parse_lit_str(nested.value()?.parse()?)?);
                    } else {
                        return Err(nested.error("index solo soporta name"));
                    }

                    Ok(())
                })?;
                config.indexes.push(index);
            } else if meta.path.is_ident("unique") {
                config.indexes.push(IndexConfig {
                    unique: true,
                    ..IndexConfig::default()
                });
            } else if meta.path.is_ident("sql_type") {
                config.sql_type = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("unsafe_sql_type") {
                config.unsafe_sql_type = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("precision") {
                config.precision = Some(parse_u8_expr(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("scale") {
                config.scale = Some(parse_u8_expr(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("computed_sql") {
                return Err(meta.error(
                    "computed_sql es un fragmento SQL unsafe; usa unsafe_computed_sql = \"...\" para reconocer explícitamente el riesgo",
                ));
            } else if meta.path.is_ident("unsafe_computed_sql") {
                config.computed_sql = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("rowversion") {
                config.rowversion = true;
            } else if meta.path.is_ident("foreign_key") {
                config.foreign_key = Some(parse_foreign_key_config(meta)?);
            } else if meta.path.is_ident("belongs_to") {
                if config.navigation.is_some() {
                    return Err(meta.error(
                        "un campo solo puede declarar una navegación: belongs_to, has_one o has_many",
                    ));
                }
                config.navigation = Some(parse_navigation_config(
                    meta,
                    NavigationKindConfig::BelongsTo,
                )?);
            } else if meta.path.is_ident("has_one") {
                if config.navigation.is_some() {
                    return Err(meta.error(
                        "un campo solo puede declarar una navegación: belongs_to, has_one o has_many",
                    ));
                }
                config.navigation =
                    Some(parse_navigation_config(meta, NavigationKindConfig::HasOne)?);
            } else if meta.path.is_ident("has_many") {
                if config.navigation.is_some() {
                    return Err(meta.error(
                        "un campo solo puede declarar una navegación: belongs_to, has_one o has_many",
                    ));
                }
                config.navigation = Some(parse_navigation_config(
                    meta,
                    NavigationKindConfig::HasMany,
                )?);
            } else if meta.path.is_ident("many_to_many") {
                return Err(meta.error(
                    "many_to_many directo no está soportado todavía; modele la relación con una entidad intermedia explícita",
                ));
            } else if meta.path.is_ident("on_delete") {
                config.on_delete = Some(parse_referential_action_expr(meta.value()?.parse()?)?);
            } else {
                return Err(meta.error("atributo orm no soportado a nivel de campo"));
            }

            Ok(())
        })?;
    }

    if config.navigation.is_some()
        && (config.column.is_some()
            || config.primary_key
            || config.identity
            || config.nullable
            || config.length.is_some()
            || config.default_sql.is_some()
            || config.renamed_from.is_some()
            || config.computed_sql.is_some()
            || config.rowversion
            || config.sql_type.is_some()
            || config.precision.is_some()
            || config.scale.is_some()
            || !config.indexes.is_empty()
            || config.foreign_key.is_some()
            || config.on_delete.is_some())
    {
        return Err(Error::new_spanned(
            field,
            "los campos de navegación solo soportan belongs_to, has_one o has_many; no se generan columnas para ellos",
        ));
    }

    if config.navigation.is_none() && is_navigation_wrapper_type(&field.ty) {
        return Err(Error::new_spanned(
            field,
            "los campos Navigation<T> y Collection<T> requieren #[orm(belongs_to(...))], #[orm(has_one(...))] o #[orm(has_many(...))]",
        ));
    }

    Ok(config)
}

fn parse_policy_field_config(field: &Field, kind: PolicyFieldsKind) -> Result<AuditFieldConfig> {
    let mut config = AuditFieldConfig::default();
    let derive_name = kind.derive_name();

    for attr in &field.attrs {
        if !attr.path().is_ident("orm") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("column") {
                config.column = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("length") {
                config.length = Some(parse_u32_expr(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("nullable") {
                config.nullable = true;
            } else if meta.path.is_ident("default_sql") {
                return Err(meta.error(
                    "default_sql es un fragmento SQL unsafe; usa unsafe_default_sql = \"...\" para reconocer explícitamente el riesgo",
                ));
            } else if meta.path.is_ident("unsafe_default_sql") {
                config.default_sql = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("renamed_from") {
                config.renamed_from = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("sql_type") {
                config.sql_type = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("unsafe_sql_type") {
                config.unsafe_sql_type = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("precision") {
                config.precision = Some(parse_u8_expr(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("scale") {
                config.scale = Some(parse_u8_expr(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("insertable") {
                config.insertable = Some(parse_bool_expr(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("updatable") {
                config.updatable = Some(parse_bool_expr(meta.value()?.parse()?)?);
            } else if (matches!(kind, PolicyFieldsKind::Audit)
                && (meta.path.is_ident("created_at")
                    || meta.path.is_ident("created_by")
                    || meta.path.is_ident("updated_at")
                    || meta.path.is_ident("updated_by")))
                || (matches!(kind, PolicyFieldsKind::SoftDelete)
                    && (meta.path.is_ident("deleted_at")
                        || meta.path.is_ident("deleted_by")
                        || meta.path.is_ident("is_deleted")))
            {
            } else {
                return Err(meta.error(format!(
                    "atributo orm no soportado en campos de {derive_name}"
                )));
            }

            Ok(())
        })?;
    }

    Ok(config)
}

fn parse_tenant_context_field_config(field: &Field) -> Result<TenantContextFieldConfig> {
    let mut config = TenantContextFieldConfig::default();

    for attr in &field.attrs {
        if !attr.path().is_ident("orm") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("column") {
                config.column = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("length") {
                config.length = Some(parse_u32_expr(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("renamed_from") {
                config.renamed_from = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("sql_type") {
                config.sql_type = Some(parse_lit_str(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("precision") {
                config.precision = Some(parse_u8_expr(meta.value()?.parse()?)?);
            } else if meta.path.is_ident("scale") {
                config.scale = Some(parse_u8_expr(meta.value()?.parse()?)?);
            } else {
                return Err(meta.error("atributo orm no soportado en campos de TenantContext"));
            }

            Ok(())
        })?;
    }

    Ok(config)
}

fn parse_lit_str(expr: Expr) -> Result<LitStr> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        }) => Ok(value),
        other => Err(Error::new_spanned(other, "se esperaba un string literal")),
    }
}

fn validate_non_empty_lit_str(value: &LitStr, message: &str) -> Result<()> {
    if value.value().is_empty() {
        return Err(Error::new_spanned(value, message));
    }

    Ok(())
}

fn parse_bool_expr(expr: Expr) -> Result<bool> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Bool(value),
            ..
        }) => Ok(value.value),
        other => Err(Error::new_spanned(other, "se esperaba un boolean literal")),
    }
}

fn parse_foreign_key_config(meta: syn::meta::ParseNestedMeta<'_>) -> Result<ForeignKeyConfig> {
    if meta.input.peek(Token![=]) {
        return parse_foreign_key_string_config(meta.value()?.parse()?);
    }

    let mut entity = None;
    let mut column = None;
    let mut name = None;

    meta.parse_nested_meta(|nested| {
        if nested.path.is_ident("entity") {
            let path: Path = nested.value()?.parse()?;
            entity = Some(path);
        } else if nested.path.is_ident("column") {
            column = Some(nested.value()?.parse::<Ident>()?);
        } else if nested.path.is_ident("name") {
            name = Some(parse_lit_str(nested.value()?.parse()?)?);
        } else {
            return Err(nested.error("foreign_key solo soporta entity, column y name"));
        }

        Ok(())
    })?;

    let entity = entity.ok_or_else(|| meta.error("foreign_key requiere entity = MiEntidad"))?;
    let column = column.ok_or_else(|| meta.error("foreign_key requiere column = id"))?;
    let generated_table_name = default_table_name_from_path(&entity)?;

    Ok(ForeignKeyConfig {
        name,
        generated_referenced_table_name: generated_table_name,
        target: ForeignKeyTarget::Structured { entity, column },
    })
}

fn parse_navigation_config(
    meta: syn::meta::ParseNestedMeta<'_>,
    kind: NavigationKindConfig,
) -> Result<NavigationConfig> {
    let content;
    syn::parenthesized!(content in meta.input);

    let target: Path = content.parse()?;
    let mut foreign_key = None;

    while !content.is_empty() {
        content.parse::<Token![,]>()?;
        if content.is_empty() {
            break;
        }

        let key: Ident = content.parse()?;
        content.parse::<Token![=]>()?;

        if key == "foreign_key" {
            if foreign_key.is_some() {
                return Err(Error::new(
                    key.span(),
                    format!("{} no permite foreign_key duplicado", kind.attribute_name()),
                ));
            }
            foreign_key = Some(content.parse()?);
        } else {
            return Err(Error::new(
                key.span(),
                format!("{} solo soporta foreign_key", kind.attribute_name()),
            ));
        }
    }

    let foreign_key = foreign_key.ok_or_else(|| {
        meta.error(format!(
            "{} requiere foreign_key = campo",
            kind.attribute_name()
        ))
    })?;

    Ok(NavigationConfig {
        kind,
        target,
        foreign_key,
    })
}

fn validate_repeated_structured_foreign_keys(
    foreign_keys: &BTreeMap<String, FieldForeignKeyInfo>,
) -> Result<()> {
    let mut targets = BTreeMap::<&str, Vec<&FieldForeignKeyInfo>>::new();

    for foreign_key in foreign_keys.values() {
        let Some(target) = foreign_key.structured_target.as_deref() else {
            continue;
        };
        targets.entry(target).or_default().push(foreign_key);
    }

    for foreign_keys_for_target in targets.values() {
        if foreign_keys_for_target.len() < 2 {
            continue;
        }

        if let Some(unnamed_foreign_key) = foreign_keys_for_target
            .iter()
            .find(|foreign_key| !foreign_key.has_explicit_name)
        {
            return Err(Error::new(
                unnamed_foreign_key.field_span,
                "múltiples foreign keys estructuradas al mismo target requieren name explícito para desambiguar navegaciones",
            ));
        }
    }

    Ok(())
}

fn parse_foreign_key_string_config(expr: Expr) -> Result<ForeignKeyConfig> {
    let value = parse_lit_str(expr)?;
    let raw = value.value();
    let segments = raw.split('.').collect::<Vec<_>>();

    let (referenced_schema, referenced_table, referenced_column) = match segments.as_slice() {
        [table, column] => (
            LitStr::new("dbo", value.span()),
            LitStr::new(table, value.span()),
            LitStr::new(column, value.span()),
        ),
        [schema, table, column] => (
            LitStr::new(schema, value.span()),
            LitStr::new(table, value.span()),
            LitStr::new(column, value.span()),
        ),
        _ => {
            return Err(Error::new_spanned(
                value,
                "foreign_key requiere el formato \"tabla.columna\" o \"schema.tabla.columna\", o la forma estructurada foreign_key(entity = Customer, column = id)",
            ));
        }
    };

    if referenced_schema.value().is_empty()
        || referenced_table.value().is_empty()
        || referenced_column.value().is_empty()
    {
        return Err(Error::new_spanned(
            value,
            "foreign_key no permite segmentos vacíos",
        ));
    }

    Ok(ForeignKeyConfig {
        name: None,
        generated_referenced_table_name: referenced_table.value(),
        target: ForeignKeyTarget::Legacy {
            referenced_schema,
            referenced_table,
            referenced_column,
        },
    })
}

fn parse_referential_action_expr(expr: Expr) -> Result<ReferentialActionConfig> {
    let value = parse_lit_str(expr)?;
    match value.value().to_ascii_lowercase().as_str() {
        "no action" => Ok(ReferentialActionConfig::NoAction),
        "cascade" => Ok(ReferentialActionConfig::Cascade),
        "set null" => Ok(ReferentialActionConfig::SetNull),
        _ => Err(Error::new_spanned(
            value,
            "solo se soportan los valores \"no action\", \"cascade\" y \"set null\"",
        )),
    }
}

fn parse_u32_expr(expr: Expr) -> Result<u32> {
    parse_int::<u32>(expr, "se esperaba un entero u32")
}

fn parse_u8_expr(expr: Expr) -> Result<u8> {
    parse_int::<u8>(expr, "se esperaba un entero u8")
}

fn parse_i64_expr(expr: Expr) -> Result<i64> {
    parse_int::<i64>(expr, "se esperaba un entero i64")
}

fn parse_int<T>(expr: Expr, message: &str) -> Result<T>
where
    T: std::str::FromStr,
    <T as std::str::FromStr>::Err: std::fmt::Display,
{
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Int(value),
            ..
        }) => value
            .base10_parse::<T>()
            .map_err(|_| Error::new_spanned(value, message)),
        other => Err(Error::new_spanned(other, message)),
    }
}

fn option_lit_str(value: Option<LitStr>) -> TokenStream2 {
    match value {
        Some(value) => quote! { Some(#value) },
        None => quote! { None },
    }
}

fn persistence_column_name_expr(
    entity: &Type,
    field_ident: &Ident,
    explicit_column: Option<&LitStr>,
) -> TokenStream2 {
    let field_name = LitStr::new(&field_ident.to_string(), field_ident.span());

    match explicit_column {
        Some(column_name) => {
            let error = LitStr::new(
                &format!(
                    "la columna '{}' no existe en la metadata de la entidad de destino",
                    column_name.value()
                ),
                column_name.span(),
            );

            quote! {{
                <#entity as ::sql_orm::core::Entity>::metadata()
                    .column(#column_name)
                    .expect(#error)
                    .column_name
            }}
        }
        None => {
            let error = LitStr::new(
                &format!(
                    "el campo '{}' no existe en la metadata de la entidad de destino",
                    field_ident
                ),
                field_ident.span(),
            );

            quote! {{
                <#entity as ::sql_orm::core::Entity>::metadata()
                    .field(#field_name)
                    .expect(#error)
                    .column_name
            }}
        }
    }
}

fn option_number<T>(value: Option<T>) -> TokenStream2
where
    T: quote::ToTokens,
{
    match value {
        Some(value) => quote! { Some(#value) },
        None => quote! { None },
    }
}

fn referential_action_tokens(action: ReferentialActionConfig) -> TokenStream2 {
    match action {
        ReferentialActionConfig::NoAction => {
            quote! { ::sql_orm::core::ReferentialAction::NoAction }
        }
        ReferentialActionConfig::Cascade => {
            quote! { ::sql_orm::core::ReferentialAction::Cascade }
        }
        ReferentialActionConfig::SetNull => {
            quote! { ::sql_orm::core::ReferentialAction::SetNull }
        }
    }
}

fn navigation_kind_tokens(kind: NavigationKindConfig) -> TokenStream2 {
    match kind {
        NavigationKindConfig::BelongsTo => quote! { ::sql_orm::core::NavigationKind::BelongsTo },
        NavigationKindConfig::HasOne => quote! { ::sql_orm::core::NavigationKind::HasOne },
        NavigationKindConfig::HasMany => quote! { ::sql_orm::core::NavigationKind::HasMany },
    }
}

fn include_navigation_impls(
    entity_ident: &Ident,
    navigations: &[PendingNavigation],
) -> Result<Vec<TokenStream2>> {
    let mut grouped =
        BTreeMap::<String, (Path, Vec<(Ident, LitStr, NavigationWrapperConfig)>)>::new();

    for navigation in navigations {
        if !matches!(
            navigation.kind,
            NavigationKindConfig::BelongsTo | NavigationKindConfig::HasOne
        ) {
            continue;
        }

        let field_ident = Ident::new(&navigation.rust_field.value(), navigation.rust_field.span());
        let target = &navigation.target;
        let key = quote! { #target }.to_string();
        grouped
            .entry(key)
            .or_insert_with(|| (navigation.target.clone(), Vec::new()))
            .1
            .push((
                field_ident,
                navigation.rust_field.clone(),
                navigation.wrapper,
            ));
    }

    grouped
        .into_values()
        .map(|(target, fields)| {
            let arms = fields.into_iter().map(|(field_ident, rust_field, wrapper)| {
                let assignment = match wrapper {
                    NavigationWrapperConfig::Eager => {
                        quote! { self.#field_ident = ::sql_orm::Navigation::from_option(value); }
                    }
                    NavigationWrapperConfig::Lazy => {
                        quote! { self.#field_ident = ::sql_orm::LazyNavigation::from_option(value); }
                    }
                };

                quote! {
                    #rust_field => {
                        #assignment
                        Ok(())
                    }
                }
            });

            Ok(quote! {
                impl ::sql_orm::IncludeNavigation<#target> for #entity_ident {
                    fn set_included_navigation(
                        &mut self,
                        navigation: &str,
                        value: ::core::option::Option<#target>,
                    ) -> ::core::result::Result<(), ::sql_orm::core::OrmError> {
                        match navigation {
                            #(#arms,)*
                            _ => Err(::sql_orm::core::OrmError::mapping(
                                ::std::format!(
                                    "entity `{}` does not support include navigation `{}` for `{}`",
                                    <Self as ::sql_orm::core::Entity>::metadata().rust_name,
                                    navigation,
                                    ::core::any::type_name::<#target>(),
                                )
                            )),
                        }
                    }
                }
            })
        })
        .collect()
}

fn include_collection_impls(
    entity_ident: &Ident,
    navigations: &[PendingNavigation],
) -> Result<Vec<TokenStream2>> {
    let mut grouped =
        BTreeMap::<String, (Path, Vec<(Ident, LitStr, NavigationWrapperConfig)>)>::new();

    for navigation in navigations {
        if !matches!(navigation.kind, NavigationKindConfig::HasMany) {
            continue;
        }

        let field_ident = Ident::new(&navigation.rust_field.value(), navigation.rust_field.span());
        let target = &navigation.target;
        let key = quote! { #target }.to_string();
        grouped
            .entry(key)
            .or_insert_with(|| (navigation.target.clone(), Vec::new()))
            .1
            .push((
                field_ident,
                navigation.rust_field.clone(),
                navigation.wrapper,
            ));
    }

    grouped
        .into_values()
        .map(|(target, fields)| {
            let arms = fields.into_iter().map(|(field_ident, rust_field, wrapper)| {
                let assignment = match wrapper {
                    NavigationWrapperConfig::Eager => {
                        quote! { self.#field_ident = ::sql_orm::Collection::from_vec(values); }
                    }
                    NavigationWrapperConfig::Lazy => {
                        quote! { self.#field_ident = ::sql_orm::LazyCollection::from_vec(values); }
                    }
                };

                quote! {
                    #rust_field => {
                        #assignment
                        Ok(())
                    }
                }
            });

            Ok(quote! {
                impl ::sql_orm::IncludeCollection<#target> for #entity_ident {
                    fn set_included_collection(
                        &mut self,
                        navigation: &str,
                        values: ::std::vec::Vec<#target>,
                    ) -> ::core::result::Result<(), ::sql_orm::core::OrmError> {
                        match navigation {
                            #(#arms,)*
                            _ => Err(::sql_orm::core::OrmError::mapping(
                                ::std::format!(
                                    "entity `{}` does not support include collection `{}` for `{}`",
                                    <Self as ::sql_orm::core::Entity>::metadata().rust_name,
                                    navigation,
                                    ::core::any::type_name::<#target>(),
                                )
                            )),
                        }
                    }
                }
            })
        })
        .collect()
}

fn validate_navigation_field_type(
    ty: &Type,
    navigation: &NavigationConfig,
) -> Result<NavigationWrapperConfig> {
    let expected_wrappers = match navigation.kind {
        NavigationKindConfig::BelongsTo | NavigationKindConfig::HasOne => {
            ("Navigation", "LazyNavigation")
        }
        NavigationKindConfig::HasMany => ("Collection", "LazyCollection"),
    };

    let (actual_target, wrapper) = navigation_wrapper_inner_last_ident(ty, expected_wrappers)
        .ok_or_else(|| {
            Error::new_spanned(
                ty,
                format!(
                    "{} requiere un campo {}<{}> o {}<{}>",
                    navigation.kind.attribute_name(),
                    expected_wrappers.0,
                    path_last_ident(&navigation.target)
                        .map(|ident| ident.to_string())
                        .unwrap_or_else(|| "Entidad".to_string()),
                    expected_wrappers.1,
                    path_last_ident(&navigation.target)
                        .map(|ident| ident.to_string())
                        .unwrap_or_else(|| "Entidad".to_string()),
                ),
            )
        })?;

    let Some(expected_target) = path_last_ident(&navigation.target) else {
        return Err(Error::new_spanned(
            &navigation.target,
            format!(
                "{} requiere una ruta de entidad válida",
                navigation.kind.attribute_name()
            ),
        ));
    };

    if actual_target != expected_target {
        return Err(Error::new_spanned(
            ty,
            format!(
                "{} apunta a {}, pero el campo usa {}",
                navigation.kind.attribute_name(),
                expected_target,
                actual_target,
            ),
        ));
    }

    Ok(wrapper)
}

fn navigation_wrapper_inner_last_ident<'a>(
    ty: &'a Type,
    wrappers: (&str, &str),
) -> Option<(&'a Ident, NavigationWrapperConfig)> {
    generic_wrapper_inner_last_ident(ty, wrappers.0)
        .map(|ident| (ident, NavigationWrapperConfig::Eager))
        .or_else(|| {
            generic_wrapper_inner_last_ident(ty, wrappers.1)
                .map(|ident| (ident, NavigationWrapperConfig::Lazy))
        })
}

fn generic_wrapper_inner_last_ident<'a>(ty: &'a Type, wrapper: &str) -> Option<&'a Ident> {
    let Type::Path(type_path) = ty else {
        return None;
    };

    let segment = type_path.path.segments.last()?;
    if segment.ident != wrapper {
        return None;
    }

    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };

    let syn::GenericArgument::Type(Type::Path(inner)) = arguments.args.first()? else {
        return None;
    };

    path_last_ident(&inner.path)
}

fn is_navigation_wrapper_type(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };

    type_path.path.segments.last().is_some_and(|segment| {
        segment.ident == "Navigation"
            || segment.ident == "Collection"
            || segment.ident == "LazyNavigation"
            || segment.ident == "LazyCollection"
    })
}

fn path_last_ident(path: &Path) -> Option<&Ident> {
    path.segments.last().map(|segment| &segment.ident)
}

fn path_key(path: &Path) -> String {
    quote! { #path }.to_string()
}

fn infer_sql_type(type_info: &TypeInfo, rowversion: bool, ty: &Type) -> Result<TokenStream2> {
    if rowversion {
        return Ok(quote! { ::sql_orm::core::SqlServerType::RowVersion });
    }

    let token = match type_info.kind {
        TypeKind::I64 => quote! { ::sql_orm::core::SqlServerType::BigInt },
        TypeKind::I32 => quote! { ::sql_orm::core::SqlServerType::Int },
        TypeKind::I16 => quote! { ::sql_orm::core::SqlServerType::SmallInt },
        TypeKind::U8 => quote! { ::sql_orm::core::SqlServerType::TinyInt },
        TypeKind::Bool => quote! { ::sql_orm::core::SqlServerType::Bit },
        TypeKind::String => quote! { ::sql_orm::core::SqlServerType::NVarChar },
        TypeKind::VecU8 => quote! { ::sql_orm::core::SqlServerType::VarBinary },
        TypeKind::Uuid => quote! { ::sql_orm::core::SqlServerType::UniqueIdentifier },
        TypeKind::NaiveDateTime => quote! { ::sql_orm::core::SqlServerType::DateTime2 },
        TypeKind::NaiveDate => quote! { ::sql_orm::core::SqlServerType::Date },
        TypeKind::NaiveTime => quote! { ::sql_orm::core::SqlServerType::Time },
        TypeKind::DateTimeFixedOffset => {
            quote! { ::sql_orm::core::SqlServerType::DateTimeOffset }
        }
        TypeKind::Decimal => quote! { ::sql_orm::core::SqlServerType::Decimal },
        TypeKind::Float => quote! { ::sql_orm::core::SqlServerType::Float },
        TypeKind::Unknown => {
            return Err(Error::new_spanned(
                ty,
                "tipo Rust no soportado todavía para derive(Entity)",
            ));
        }
    };

    Ok(token)
}

fn sql_type_from_string(value: &LitStr) -> Result<TokenStream2> {
    sql_type_from_string_with_custom(value, false)
}

fn unsafe_sql_type_from_string(value: &LitStr) -> TokenStream2 {
    sql_type_from_string_with_custom(value, true)
        .expect("unsafe SQL type fragments always allow custom SQL Server types")
}

fn sql_type_from_string_with_custom(value: &LitStr, allow_custom: bool) -> Result<TokenStream2> {
    let normalized = value.value().to_ascii_lowercase();

    let token = if normalized.starts_with("bigint") {
        quote! { ::sql_orm::core::SqlServerType::BigInt }
    } else if normalized == "int" {
        quote! { ::sql_orm::core::SqlServerType::Int }
    } else if normalized.starts_with("smallint") {
        quote! { ::sql_orm::core::SqlServerType::SmallInt }
    } else if normalized.starts_with("tinyint") {
        quote! { ::sql_orm::core::SqlServerType::TinyInt }
    } else if normalized.starts_with("bit") {
        quote! { ::sql_orm::core::SqlServerType::Bit }
    } else if normalized.starts_with("uniqueidentifier") {
        quote! { ::sql_orm::core::SqlServerType::UniqueIdentifier }
    } else if normalized.starts_with("datetimeoffset") {
        quote! { ::sql_orm::core::SqlServerType::DateTimeOffset }
    } else if normalized.starts_with("datetime2") {
        quote! { ::sql_orm::core::SqlServerType::DateTime2 }
    } else if normalized.starts_with("date") {
        quote! { ::sql_orm::core::SqlServerType::Date }
    } else if normalized.starts_with("time") {
        quote! { ::sql_orm::core::SqlServerType::Time }
    } else if normalized.starts_with("decimal") {
        quote! { ::sql_orm::core::SqlServerType::Decimal }
    } else if normalized.starts_with("float") {
        quote! { ::sql_orm::core::SqlServerType::Float }
    } else if normalized.starts_with("money") {
        quote! { ::sql_orm::core::SqlServerType::Money }
    } else if normalized.starts_with("nvarchar") {
        quote! { ::sql_orm::core::SqlServerType::NVarChar }
    } else if normalized.starts_with("varbinary") {
        quote! { ::sql_orm::core::SqlServerType::VarBinary }
    } else if normalized.starts_with("rowversion") {
        quote! { ::sql_orm::core::SqlServerType::RowVersion }
    } else if allow_custom {
        quote! { ::sql_orm::core::SqlServerType::Custom(#value) }
    } else {
        return Err(Error::new_spanned(
            value,
            "sql_type no reconoce este tipo SQL Server; usa unsafe_sql_type = \"...\" para tipos SQL custom",
        ));
    };

    Ok(token)
}

fn analyze_type(ty: &Type) -> Result<TypeInfo> {
    let nullable = option_inner_type(ty).is_some();
    let effective = option_inner_type(ty).unwrap_or(ty);
    let kind = classify_type(effective)?;

    Ok(TypeInfo {
        nullable,
        is_integer: matches!(
            kind,
            TypeKind::I64 | TypeKind::I32 | TypeKind::I16 | TypeKind::U8
        ),
        is_vec_u8: matches!(kind, TypeKind::VecU8),
        default_max_length: matches!(kind, TypeKind::String).then_some(255),
        default_precision: matches!(kind, TypeKind::Decimal).then_some(18),
        default_scale: matches!(kind, TypeKind::Decimal).then_some(2),
        kind,
    })
}

fn classify_type(ty: &Type) -> Result<TypeKind> {
    match ty {
        Type::Path(type_path) => {
            let segment = type_path
                .path
                .segments
                .last()
                .ok_or_else(|| Error::new_spanned(type_path, "tipo inválido"))?;

            let ident = segment.ident.to_string();
            let kind = match ident.as_str() {
                "i64" => TypeKind::I64,
                "i32" => TypeKind::I32,
                "i16" => TypeKind::I16,
                "u8" => TypeKind::U8,
                "bool" => TypeKind::Bool,
                "String" => TypeKind::String,
                "Uuid" => TypeKind::Uuid,
                "NaiveDateTime" => TypeKind::NaiveDateTime,
                "NaiveDate" => TypeKind::NaiveDate,
                "NaiveTime" => TypeKind::NaiveTime,
                "DateTime" if type_path_is_datetime_fixed_offset(&type_path.path) => {
                    TypeKind::DateTimeFixedOffset
                }
                "Decimal" => TypeKind::Decimal,
                "f32" | "f64" => TypeKind::Float,
                "Vec" if type_path_is_vec_u8(&type_path.path) => TypeKind::VecU8,
                _ => TypeKind::Unknown,
            };

            Ok(kind)
        }
        _ => Ok(TypeKind::Unknown),
    }
}

fn option_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };

    let segment = type_path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }

    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };

    let syn::GenericArgument::Type(inner) = arguments.args.first()? else {
        return None;
    };

    Some(inner)
}

fn dbset_entity_type(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };

    let segment = type_path.path.segments.last()?;
    if segment.ident != "DbSet" {
        return None;
    }

    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };

    let syn::GenericArgument::Type(inner) = arguments.args.first()? else {
        return None;
    };

    Some(inner)
}

fn type_path_is_vec_u8(path: &Path) -> bool {
    let Some(segment) = path.segments.last() else {
        return false;
    };

    if segment.ident != "Vec" {
        return false;
    }

    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return false;
    };

    let Some(syn::GenericArgument::Type(Type::Path(inner_path))) = arguments.args.first() else {
        return false;
    };

    inner_path.path.is_ident("u8")
}

fn type_path_is_datetime_fixed_offset(path: &Path) -> bool {
    let Some(segment) = path.segments.last() else {
        return false;
    };

    if segment.ident != "DateTime" {
        return false;
    }

    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return false;
    };

    let Some(syn::GenericArgument::Type(Type::Path(inner_path))) = arguments.args.first() else {
        return false;
    };

    inner_path
        .path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "FixedOffset")
}

fn default_table_name(ident: &Ident) -> String {
    pluralize(&to_snake_case(&ident.to_string()))
}

fn default_table_name_from_path(path: &Path) -> Result<String> {
    let ident = path
        .segments
        .last()
        .map(|segment| &segment.ident)
        .ok_or_else(|| {
            Error::new_spanned(path, "foreign_key requiere una ruta de entidad válida")
        })?;

    Ok(default_table_name(ident))
}

fn to_snake_case(value: &str) -> String {
    let mut output = String::with_capacity(value.len());

    for (index, ch) in value.chars().enumerate() {
        if ch.is_uppercase() {
            if index > 0 {
                output.push('_');
            }

            for lower in ch.to_lowercase() {
                output.push(lower);
            }
        } else {
            output.push(ch);
        }
    }

    output
}

fn pluralize(value: &str) -> String {
    if ends_with_consonant_y(value) {
        let stem = &value[..value.len() - 1];
        format!("{stem}ies")
    } else if value.ends_with('s')
        || value.ends_with('x')
        || value.ends_with('z')
        || value.ends_with("ch")
        || value.ends_with("sh")
    {
        format!("{value}es")
    } else {
        format!("{value}s")
    }
}

fn ends_with_consonant_y(value: &str) -> bool {
    let mut chars = value.chars().rev();
    let Some(last) = chars.next() else {
        return false;
    };
    let Some(previous) = chars.next() else {
        return false;
    };

    last == 'y' && !matches!(previous, 'a' | 'e' | 'i' | 'o' | 'u')
}

fn generated_index_name(prefix: &str, table: &str, column: &str, span: Span) -> LitStr {
    LitStr::new(&format!("{prefix}_{table}_{column}"), span)
}

fn generated_foreign_key_name(
    table: &str,
    column: &str,
    referenced_table: &str,
    span: Span,
) -> LitStr {
    LitStr::new(&format!("fk_{table}_{column}_{referenced_table}"), span)
}

#[derive(Default)]
struct EntityConfig {
    table: Option<LitStr>,
    schema: Option<LitStr>,
    renamed_from: Option<LitStr>,
    indexes: Vec<EntityIndexConfig>,
    audit: Option<Path>,
    soft_delete: Option<Path>,
    tenant: Option<Path>,
}

#[derive(Default)]
struct EntityIndexConfig {
    name: Option<LitStr>,
    unique: bool,
    columns: Vec<Ident>,
}

#[derive(Default)]
struct PersistenceModelConfig {
    entity: Option<Type>,
}

#[derive(Default)]
struct PersistenceFieldConfig {
    column: Option<LitStr>,
}

#[derive(Default)]
struct AuditFieldConfig {
    column: Option<LitStr>,
    renamed_from: Option<LitStr>,
    nullable: bool,
    length: Option<u32>,
    default_sql: Option<LitStr>,
    sql_type: Option<LitStr>,
    unsafe_sql_type: Option<LitStr>,
    precision: Option<u8>,
    scale: Option<u8>,
    insertable: Option<bool>,
    updatable: Option<bool>,
}

#[derive(Default)]
struct TenantContextFieldConfig {
    column: Option<LitStr>,
    renamed_from: Option<LitStr>,
    length: Option<u32>,
    sql_type: Option<LitStr>,
    unsafe_sql_type: Option<LitStr>,
    precision: Option<u8>,
    scale: Option<u8>,
}

#[derive(Default)]
struct FieldConfig {
    column: Option<LitStr>,
    renamed_from: Option<LitStr>,
    primary_key: bool,
    identity: bool,
    identity_seed: Option<i64>,
    identity_increment: Option<i64>,
    nullable: bool,
    length: Option<u32>,
    default_sql: Option<LitStr>,
    computed_sql: Option<LitStr>,
    rowversion: bool,
    sql_type: Option<LitStr>,
    unsafe_sql_type: Option<LitStr>,
    precision: Option<u8>,
    scale: Option<u8>,
    indexes: Vec<IndexConfig>,
    foreign_key: Option<ForeignKeyConfig>,
    on_delete: Option<ReferentialActionConfig>,
    navigation: Option<NavigationConfig>,
}

#[derive(Default)]
struct IndexConfig {
    name: Option<LitStr>,
    unique: bool,
}

struct ForeignKeyConfig {
    name: Option<LitStr>,
    generated_referenced_table_name: String,
    target: ForeignKeyTarget,
}

struct FieldForeignKeyInfo {
    name: LitStr,
    local_column: LitStr,
    referenced_column: TokenStream2,
    field_span: Span,
    has_explicit_name: bool,
    structured_target: Option<String>,
}

struct PendingNavigation {
    rust_field: LitStr,
    kind: NavigationKindConfig,
    kind_tokens: TokenStream2,
    wrapper: NavigationWrapperConfig,
    target: Path,
    target_rust_name: LitStr,
    foreign_key_field: Ident,
    foreign_key_field_name: String,
}

struct NavigationConfig {
    kind: NavigationKindConfig,
    target: Path,
    foreign_key: Ident,
}

#[derive(Clone, Copy)]
enum NavigationKindConfig {
    BelongsTo,
    HasOne,
    HasMany,
}

#[derive(Clone, Copy)]
enum NavigationWrapperConfig {
    Eager,
    Lazy,
}

impl NavigationKindConfig {
    fn attribute_name(self) -> &'static str {
        match self {
            Self::BelongsTo => "belongs_to",
            Self::HasOne => "has_one",
            Self::HasMany => "has_many",
        }
    }
}

impl ForeignKeyConfig {
    fn structured_target_key(&self) -> Option<String> {
        match &self.target {
            ForeignKeyTarget::Structured { entity, .. } => Some(path_key(entity)),
            ForeignKeyTarget::Legacy { .. } => None,
        }
    }

    fn referenced_schema_tokens(&self) -> TokenStream2 {
        match &self.target {
            ForeignKeyTarget::Legacy {
                referenced_schema, ..
            } => quote! { #referenced_schema },
            ForeignKeyTarget::Structured { entity, .. } => {
                quote! { #entity::__SQL_ORM_ENTITY_SCHEMA }
            }
        }
    }

    fn referenced_table_tokens(&self) -> TokenStream2 {
        match &self.target {
            ForeignKeyTarget::Legacy {
                referenced_table, ..
            } => quote! { #referenced_table },
            ForeignKeyTarget::Structured { entity, .. } => {
                quote! { #entity::__SQL_ORM_ENTITY_TABLE }
            }
        }
    }

    fn referenced_column_tokens(&self) -> TokenStream2 {
        match &self.target {
            ForeignKeyTarget::Legacy {
                referenced_column, ..
            } => quote! { #referenced_column },
            ForeignKeyTarget::Structured { entity, column } => {
                quote! { #entity::#column.column_name() }
            }
        }
    }
}

enum ForeignKeyTarget {
    Legacy {
        referenced_schema: LitStr,
        referenced_table: LitStr,
        referenced_column: LitStr,
    },
    Structured {
        entity: Path,
        column: Ident,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReferentialActionConfig {
    NoAction,
    Cascade,
    SetNull,
}

struct TypeInfo {
    nullable: bool,
    is_integer: bool,
    is_vec_u8: bool,
    default_max_length: Option<u32>,
    default_precision: Option<u8>,
    default_scale: Option<u8>,
    kind: TypeKind,
}

enum TypeKind {
    I64,
    I32,
    I16,
    U8,
    Bool,
    String,
    VecU8,
    Uuid,
    NaiveDateTime,
    NaiveDate,
    NaiveTime,
    DateTimeFixedOffset,
    Decimal,
    Float,
    Unknown,
}
