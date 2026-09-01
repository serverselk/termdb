CREATE TABLE IF NOT EXISTS customers (
    id   serial PRIMARY KEY,
    name text NOT NULL,
    status text
);
CREATE TABLE IF NOT EXISTS orders (
    id          serial PRIMARY KEY,
    customer_id integer REFERENCES customers (id),
    total       numeric(12, 2)
);
CREATE TABLE IF NOT EXISTS products (
    id    serial PRIMARY KEY,
    sku   text UNIQUE NOT NULL,
    price numeric(10, 2)
);

INSERT INTO customers (name, status)
SELECT 'Ada Lovelace', 'active'
WHERE NOT EXISTS (SELECT 1 FROM customers);
INSERT INTO products (sku, price)
SELECT 'SKU-0001', 19.99
WHERE NOT EXISTS (SELECT 1 FROM products);