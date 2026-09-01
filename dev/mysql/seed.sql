USE mysql_test;

CREATE TABLE IF NOT EXISTS customers (
    id   INT PRIMARY KEY AUTO_INCREMENT,
    name VARCHAR(255) NOT NULL
);

INSERT IGNORE INTO customers (id, name) VALUES (1, 'Ada Lovelace');