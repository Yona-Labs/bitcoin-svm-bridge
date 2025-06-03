CREATE TABLE utxos
(
    txid          BLOB,
    vout          INTEGER,
    amount        INTEGER,
    script_pubkey BLOB,
    yona_address  VARCHAR(255),
    bridge_pubkey BLOB,
    redeem_script BLOB,
    PRIMARY KEY (txid, vout)
)