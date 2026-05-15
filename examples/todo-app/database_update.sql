-- sql-orm database update

SET ANSI_NULLS ON;

SET ANSI_PADDING ON;

SET ANSI_WARNINGS ON;

SET ARITHABORT ON;

SET CONCAT_NULL_YIELDS_NULL ON;

SET QUOTED_IDENTIFIER ON;

SET NUMERIC_ROUNDABORT OFF;

IF OBJECT_ID(N'dbo.__sql_orm_migrations', N'U') IS NULL
BEGIN
    CREATE TABLE [dbo].[__sql_orm_migrations] (
        [id] nvarchar(150) NOT NULL PRIMARY KEY,
        [name] nvarchar(255) NOT NULL,
        [applied_at] datetime2 NOT NULL DEFAULT SYSUTCDATETIME(),
        [checksum] nvarchar(128) NOT NULL,
        [orm_version] nvarchar(50) NOT NULL
    );
END

IF EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations] WHERE [id] = N'1777037241551380209_createtodoschema' AND [checksum] <> N'05186f7e590c6ba5')
BEGIN
    THROW 50001, N'sql-orm migration checksum mismatch for 1777037241551380209_createtodoschema', 1;
END

IF NOT EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations] WHERE [id] = N'1777037241551380209_createtodoschema')
BEGIN
    BEGIN TRY
        BEGIN TRANSACTION;
    EXEC(N'IF SCHEMA_ID(N''todo'') IS NULL EXEC(N''CREATE SCHEMA [todo]'');');
    EXEC(N'CREATE TABLE [todo].[todo_items] (
    [id] bigint IDENTITY(1, 1) NOT NULL,
    [list_id] bigint NOT NULL,
    [created_by_user_id] bigint NOT NULL,
    [completed_by_user_id] bigint NULL,
    [title] nvarchar(200) NOT NULL,
    [position] int NOT NULL,
    [is_completed] bit NOT NULL DEFAULT 0,
    [completed_at] nvarchar(255) NULL,
    [created_at] nvarchar(255) NOT NULL DEFAULT SYSUTCDATETIME(),
    [version] rowversion,
    PRIMARY KEY ([id])
);');
    EXEC(N'CREATE TABLE [todo].[todo_lists] (
    [id] bigint IDENTITY(1, 1) NOT NULL,
    [owner_user_id] bigint NOT NULL,
    [title] nvarchar(160) NOT NULL,
    [is_archived] bit NOT NULL DEFAULT 0,
    [created_at] nvarchar(255) NOT NULL DEFAULT SYSUTCDATETIME(),
    [version] rowversion,
    PRIMARY KEY ([id])
);');
    EXEC(N'CREATE TABLE [todo].[users] (
    [id] bigint IDENTITY(1, 1) NOT NULL,
    [email] nvarchar(180) NOT NULL,
    [display_name] nvarchar(120) NOT NULL,
    [created_at] nvarchar(255) NOT NULL DEFAULT SYSUTCDATETIME(),
    [version] rowversion,
    PRIMARY KEY ([id])
);');
    EXEC(N'CREATE INDEX [ix_todo_items_list_position] ON [todo].[todo_items] ([list_id] ASC, [position] ASC);');
    EXEC(N'ALTER TABLE [todo].[todo_items] ADD CONSTRAINT [fk_todo_items_list_id_todo_lists] FOREIGN KEY ([list_id]) REFERENCES [todo].[todo_lists] ([id]) ON DELETE CASCADE ON UPDATE NO ACTION;');
    EXEC(N'ALTER TABLE [todo].[todo_items] ADD CONSTRAINT [fk_todo_items_created_by_user_id_users] FOREIGN KEY ([created_by_user_id]) REFERENCES [todo].[users] ([id]) ON DELETE NO ACTION ON UPDATE NO ACTION;');
    EXEC(N'ALTER TABLE [todo].[todo_items] ADD CONSTRAINT [fk_todo_items_completed_by_user_id_users] FOREIGN KEY ([completed_by_user_id]) REFERENCES [todo].[users] ([id]) ON DELETE NO ACTION ON UPDATE NO ACTION;');
    EXEC(N'CREATE INDEX [ix_todo_lists_owner_title] ON [todo].[todo_lists] ([owner_user_id] ASC, [title] ASC);');
    EXEC(N'ALTER TABLE [todo].[todo_lists] ADD CONSTRAINT [fk_todo_lists_owner_user_id_users] FOREIGN KEY ([owner_user_id]) REFERENCES [todo].[users] ([id]) ON DELETE CASCADE ON UPDATE NO ACTION;');
    EXEC(N'CREATE UNIQUE INDEX [ux_users_email] ON [todo].[users] ([email] ASC);');
        INSERT INTO [dbo].[__sql_orm_migrations] ([id], [name], [checksum], [orm_version]) VALUES (N'1777037241551380209_createtodoschema', N'createtodoschema', N'05186f7e590c6ba5', N'0.1.0');
        COMMIT TRANSACTION;
    END TRY
    BEGIN CATCH
        IF XACT_STATE() <> 0
            ROLLBACK TRANSACTION;
        THROW;
    END CATCH
END

IF EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations] WHERE [id] = N'1777037241765419900_verifytodoschemanoop' AND [checksum] <> N'b4f0235ecab4bbb2')
BEGIN
    THROW 50001, N'sql-orm migration checksum mismatch for 1777037241765419900_verifytodoschemanoop', 1;
END

IF NOT EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations] WHERE [id] = N'1777037241765419900_verifytodoschemanoop')
BEGIN
    BEGIN TRY
        BEGIN TRANSACTION;
        INSERT INTO [dbo].[__sql_orm_migrations] ([id], [name], [checksum], [orm_version]) VALUES (N'1777037241765419900_verifytodoschemanoop', N'verifytodoschemanoop', N'b4f0235ecab4bbb2', N'0.1.0');
        COMMIT TRANSACTION;
    END TRY
    BEGIN CATCH
        IF XACT_STATE() <> 0
            ROLLBACK TRANSACTION;
        THROW;
    END CATCH
END

IF EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations] WHERE [id] = N'1777038734516571446_addtodolistdescription' AND [checksum] <> N'c924fbfe5e5dcba9')
BEGIN
    THROW 50001, N'sql-orm migration checksum mismatch for 1777038734516571446_addtodolistdescription', 1;
END

IF NOT EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations] WHERE [id] = N'1777038734516571446_addtodolistdescription')
BEGIN
    BEGIN TRY
        BEGIN TRANSACTION;
    EXEC(N'ALTER TABLE [todo].[todo_lists] ADD [description] nvarchar(500) NULL;');
        INSERT INTO [dbo].[__sql_orm_migrations] ([id], [name], [checksum], [orm_version]) VALUES (N'1777038734516571446_addtodolistdescription', N'addtodolistdescription', N'c924fbfe5e5dcba9', N'0.1.0');
        COMMIT TRANSACTION;
    END TRY
    BEGIN CATCH
        IF XACT_STATE() <> 0
            ROLLBACK TRANSACTION;
        THROW;
    END CATCH
END
