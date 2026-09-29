-- Run only in the dedicated tabularis_pr31_test database.
-- Existing fixture tables are preserved; rerunning this file does not reset data.
IF DB_NAME() <> N'tabularis_pr31_test'
    THROW 50000, 'Select tabularis_pr31_test before loading this fixture.', 1;

IF SCHEMA_ID(N'pr31') IS NULL
    EXEC(N'CREATE SCHEMA [pr31]');

IF OBJECT_ID(N'[pr31].[orders]', N'U') IS NULL
BEGIN
    -- Deliberately not IDENTITY: all generated UPDATE assignments are writable.
    CREATE TABLE [pr31].[orders] (
        [id] INT NOT NULL PRIMARY KEY,
        [order status] NVARCHAR(30) NOT NULL,
        [a b] INT NOT NULL,
        [a-b] INT NOT NULL
    );
    ;WITH numbers AS (
        SELECT 1 AS n
        UNION ALL
        SELECT n + 1 FROM numbers WHERE n < 150
    )
    INSERT INTO [pr31].[orders] ([id], [order status], [a b], [a-b])
    SELECT n, N'open', n * 10, n * 100 FROM numbers
    OPTION (MAXRECURSION 150);
END;

IF OBJECT_ID(N'[pr31].[order]]details]', N'U') IS NULL
BEGIN
    CREATE TABLE [pr31].[order]]details] (
        [id] INT NOT NULL PRIMARY KEY,
        [order]]status] NVARCHAR(30) NOT NULL
    );
    INSERT INTO [pr31].[order]]details] VALUES (1, N'open'), (2, N'closed');
END;

SELECT COUNT(*) AS fixture_rows FROM [pr31].[orders];
