CREATE TABLE IF NOT EXISTS customers (
    id         serial PRIMARY KEY,
    name       varchar(120)  NOT NULL,
    email      varchar(160)  NOT NULL,
    city       varchar(80),
    is_active  boolean       DEFAULT true,
    created_at timestamptz   DEFAULT now(),
    notes      text
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

INSERT INTO customers (name, email, city, is_active, notes)
SELECT 'Customer ' || i,
       'customer' || i || '@example.com',
       (ARRAY['Lisbon', 'Porto', 'Coimbra'])[1 + (i % 3)],
       (i % 2 = 0),
       CASE WHEN i % 5 = 0 THEN NULL ELSE 'note ' || i END
FROM generate_series(1, 25) AS i
WHERE NOT EXISTS (SELECT 1 FROM customers);

INSERT INTO products (sku, price)
SELECT 'SKU-' || to_char(i, 'FM0000'), (i * 1.99)::numeric(10, 2)
FROM generate_series(1, 10) AS i
WHERE NOT EXISTS (SELECT 1 FROM products);