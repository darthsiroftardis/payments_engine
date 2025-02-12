use std::collections::{BTreeMap, BTreeSet};
use std::error;
use std::fmt::Formatter;
use std::io::{self};
use std::process;

use anyhow;
use csv::ReaderBuilder;
use serde::{Deserialize, Serialize};

#[derive(Debug)]
enum Error {
    LockedAccount,
    InvalidDeposit,
    InvalidWithdrawal,
    MissingUser,
    DisputeUserIDMismatch,
    NoAmountOnPrevTx,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::LockedAccount => {
                write!(f, "Attempted to mutate a locked account")
            }
            Error::InvalidDeposit => {
                write!(f, "Deposit transaction did not specify an amount")
            }
            Error::InvalidWithdrawal => {
                write!(f, "Withdrawal transaction did not specify an amount")
            }
            Error::MissingUser => {
                write!(f, "Referred to a previous transaction which does not have a corresponding user")
            }
            Error::DisputeUserIDMismatch => {
                write!(f, "The disputes a client id which is different than previously recorded")
            }
            Error::NoAmountOnPrevTx => {
                write!(f, "The disputed transaction did not specify an amount to hold")
            }
        }
    }
}

impl error::Error for Error {}

#[derive(Debug, Copy, Clone, PartialEq)]
struct User {
    id: u16,
    available: f64,
    held: f64,
    locked: bool,
}

impl User {
    fn new(id: u16, available: f64) -> Self {
        Self {
            id,
            available,
            held: 0.0,
            locked: false,
        }
    }

    #[cfg(test)]
    fn new_test_user(id: u16, available: f64, held: f64, locked: bool) -> Self {
        Self {
            id,
            available,
            held,
            locked,
        }
    }

    fn total(&self) -> f64 {
        self.available + self.held
    }

    fn apply_deposit(&mut self, amount: f64) -> Result<(), Error> {
        if self.locked {
            return Err(Error::LockedAccount)
        }
        self.available += amount;
        Ok(())
    }

    fn apply_withdrawal(&mut self, amount: f64) -> Result<(), Error> {
        if self.locked {
            return Err(Error::LockedAccount)
        }
        if self.available >= amount {
            self.available -= amount
        }
        Ok(())
    }

    fn apply_chargeback(&mut self, hold_amount: f64) -> Result<(), Error> {
        if self.held < hold_amount {
            self.lock_user();
            return Err(Error::LockedAccount)
        }
        self.held -= hold_amount;
        Ok(())
    }

    fn release_hold(&mut self, amount: f64) {
        self.held -= amount;
        self.available += amount;
    }

    fn apply_hold(&mut self, hold_amount: f64) {
        self.available -= hold_amount;
        self.held += hold_amount;
    }

    fn lock_user(&mut self) {
        self.locked = true
    }
}

#[derive(Debug, Deserialize, Clone, Copy)]
enum TransactionType {
    #[serde(rename = "deposit")]
    Deposit,
    #[serde(rename = "withdrawal")]
    Withdrawal,
    #[serde(rename = "dispute")]
    Dispute,
    #[serde(rename = "resolve")]
    Resolve,
    #[serde(rename = "chargeback")]
    Chargeback,
}

#[derive(Debug, Deserialize, Copy, Clone)]
struct Transaction {
    #[serde(rename = "type")]
    transaction_type: TransactionType,
    #[serde(rename = "client")]
    client_id: u16,
    #[serde(rename = "tx")]
    id: u32,
    amount: Option<f64>,
}

impl Transaction {
    #[cfg(test)]
    fn new(
        transaction_type: TransactionType,
        client_id: u16,
        id: u32,
        amount: Option<f64>,
    ) -> Self {
        Self {
            transaction_type,
            client_id,
            id,
            amount,
        }
    }

    fn client_id(&self) -> u16 {
        self.client_id
    }

    fn transaction_type(&self) -> TransactionType {
        self.transaction_type
    }
}

#[derive(Debug, Serialize, Default)]
struct ClientAccount {
    client: u16,
    available: f64,
    held: f64,
    total: f64,
    locked: bool,
}

impl From<User> for ClientAccount {
    fn from(value: User) -> Self {
        let client = value.id;
        let available = value.available;
        let held = value.held;
        let total = value.total();
        let locked = value.locked;

        Self {
            client,
            available,
            held,
            total,
            locked,
        }
    }
}

fn process_transactions(transactions: Vec<Transaction>) -> Result<BTreeMap<u16, User>, Error> {
    let mut users = BTreeMap::<u16, User>::new();
    let mut deposits = BTreeMap::new();
    let mut disputes = BTreeSet::new();

    for transaction in transactions {
        let transaction_id = transaction.id;
        let user_id = transaction.client_id();
        match transaction.transaction_type() {
            TransactionType::Deposit => {
                let amount = match transaction.amount {
                    Some(amount) => amount,
                    None => return Err(Error::InvalidDeposit),
                };
                match users.get_mut(&user_id) {
                    None => {
                        users.insert(user_id, User::new(user_id, amount));
                    }
                    Some(user) => user.apply_deposit(amount)?,
                };
                deposits.insert(transaction_id, transaction);
            }
            TransactionType::Withdrawal => {
                let user_id = transaction.client_id();
                let amount = match transaction.amount {
                    Some(amount) => amount,
                    None => return Err(Error::InvalidWithdrawal),
                };
                match users.get_mut(&user_id) {
                    None => return Err(Error::MissingUser),
                    Some(user) => user.apply_withdrawal(amount)?,
                };
            }
            TransactionType::Dispute => {
                if let Some(disputed) = deposits.get(&transaction_id) {
                    let disputed_amount = match disputed.amount {
                        Some(amount) => amount,
                        None => return Err(Error::NoAmountOnPrevTx),
                    };
                    if disputed.client_id() != user_id {
                        return Err(Error::DisputeUserIDMismatch);
                    }

                    match users.get_mut(&user_id) {
                        None => return Err(Error::MissingUser),
                        Some(user) => user.apply_hold(disputed_amount),
                    };
                    disputes.insert(transaction_id);
                }
            }
            TransactionType::Resolve => {
                if !disputes.contains(&transaction_id) {
                    continue;
                }
                if let Some(disputed) = deposits.get(&transaction_id) {
                    let disputed_amount = match disputed.amount {
                        Some(amount) => amount,
                        None => return Err(Error::NoAmountOnPrevTx),
                    };
                    if disputed.client_id() != user_id {
                        return Err(Error::DisputeUserIDMismatch);
                    }

                    match users.get_mut(&user_id) {
                        None => return Err(Error::MissingUser),
                        Some(user) => user.release_hold(disputed_amount),
                    };
                    disputes.remove(&transaction_id);
                }
            }
            TransactionType::Chargeback => {
                if let Some(chargeback) = deposits.get(&transaction_id) {
                    let charge_back_amount = match chargeback.amount {
                        Some(amount) => amount,
                        None => return Err(Error::NoAmountOnPrevTx),
                    };
                    if chargeback.client_id() != user_id {
                        return Err(Error::DisputeUserIDMismatch);
                    }

                    match users.get_mut(&user_id) {
                        None => return Err(Error::MissingUser),
                        Some(user) => {
                            user.apply_chargeback(charge_back_amount)?;
                            user.lock_user()
                        }
                    };
                }
            }
        }
    }

    Ok(users)
}

fn read_file(file_path: String) -> anyhow::Result<Vec<Transaction>> {
    let mut rdr = ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_path(file_path)?;
    let mut transactions = vec![];
    for result in rdr.deserialize() {
        // Notice that we need to provide a type hint for automatic
        // deserialization.
        let transaction: Transaction = result?;
        transactions.push(transaction);
    }
    Ok(transactions)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: cargo run -- <transactions.csv");
        return Ok(());
    }

    let file_path = &args[1];
    match read_file(file_path.clone()) {
        Err(err) => {
            println!("error reading file: {}", err);
            process::exit(1);
        }
        Ok(transactions) => {
            let users = process_transactions(transactions)?;
            let mut wtr = csv::Writer::from_writer(io::stdout());

            for user in users.values() {
                wtr.serialize(ClientAccount::from(*user))?;
            }
            wtr.flush()?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_parse_transactions() {
        let tx_1 = Transaction::new(TransactionType::Deposit, 1, 1, Some(1.0));
        let tx_2 = Transaction::new(TransactionType::Deposit, 2, 2, Some(2.0));
        let tx_3 = Transaction::new(TransactionType::Deposit, 1, 3, Some(2.0));
        let tx_4 = Transaction::new(TransactionType::Withdrawal, 1, 4, Some(1.5));
        let tx_5 = Transaction::new(TransactionType::Withdrawal, 2, 5, Some(3.0));
        let transactions = vec![tx_1, tx_2, tx_3, tx_4, tx_5];

        let expected_users = {
            let mut ret = BTreeMap::new();
            let user_1 = User::new_test_user(1, 1.5, 0.0, false);
            ret.insert(1u16, user_1);
            let user_2 = User::new_test_user(2, 2.0, 0.0, false);
            ret.insert(2u16, user_2);
            ret
        };

        let actual_users =
            process_transactions(transactions).expect("must work based on test case");
        assert_eq!(expected_users, actual_users)
    }

    #[test]
    fn should_correctly_hold_based_on_dispute() {
        let deposit_1= Transaction::new(TransactionType::Deposit, 1, 1, Some(10.0));
        let deposit_2 =  Transaction::new(TransactionType::Deposit, 1, 2, Some(5.0));
        let dispute = Transaction::new(TransactionType::Dispute, 1, 2, None);

        let transactions =  vec![deposit_1, deposit_2, dispute];

        let actual_users =
            process_transactions(transactions).expect("must work based on test case");

        let user = *actual_users.get(&1)
            .expect("there must be at least one user");


        assert_eq!(user.available, 10.0);
        assert_eq!(user.held, 5.0);
        assert_eq!(user.total(), 15.0);
    }

    #[test]
    fn should_correctly_hold_based_on_dispute_and_release_based_on_resolve() {
        let deposit_1= Transaction::new(TransactionType::Deposit, 1, 1, Some(10.0));
        let deposit_2 =  Transaction::new(TransactionType::Deposit, 1, 2, Some(5.0));
        let dispute = Transaction::new(TransactionType::Dispute, 1, 2, None);

        let transactions =  vec![deposit_1, deposit_2, dispute];

        let actual_users =
            process_transactions(transactions).expect("must work based on test case");

        let user = *actual_users.get(&1)
            .expect("there must be at least one user");


        assert_eq!(user.available, 10.0);
        assert_eq!(user.held, 5.0);
        assert_eq!(user.total(), 15.0);

        let resolve = Transaction::new(TransactionType::Resolve, 1, 2, None);

        let transactions =  vec![deposit_1, deposit_2, dispute, resolve];

        let actual_users =
            process_transactions(transactions).expect("must work based on test case");

        let user = *actual_users.get(&1)
            .expect("there must be at least one user");

        assert_eq!(user.available, 15.0);
        assert_eq!(user.held, 0.0);
        assert_eq!(user.total(), 15.0);
    }


    #[test]
    fn should_correctly_chargeback() {
        let deposit_1= Transaction::new(TransactionType::Deposit, 1, 1, Some(10.0));
        let deposit_2 =  Transaction::new(TransactionType::Deposit, 1, 2, Some(5.0));
        let dispute = Transaction::new(TransactionType::Dispute, 1, 2, None);
        let charge_back = Transaction::new(TransactionType::Chargeback, 1, 2, None);

        let transactions =  vec![deposit_1, deposit_2, dispute, charge_back];

        let actual_users =
            process_transactions(transactions).expect("must work based on test case");

        let user = *actual_users.get(&1)
            .expect("there must be at least one user");

        assert_eq!(user.available, 10.0);
        assert_eq!(user.held, 0.0);
        assert_eq!(user.total(), 10.0);
    }

}
