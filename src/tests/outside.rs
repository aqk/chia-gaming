use std::collections::{HashMap, VecDeque};

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::channel_handler::runner::channel_handler_env;
use crate::channel_handler::types::{ChannelHandlerEnv, ChannelHandlerPrivateKeys, ReadableMove};
use crate::common::standard_coin::{private_to_public_key, puzzle_hash_for_pk};
use crate::common::types::{
    AllocEncoder, Amount, CoinID, CoinString, Error, GameID, PrivateKey, PuzzleHash, Spend,
    SpendBundle, Timeout,
};
use crate::outside::{
    BootstrapTowardWallet, PacketSender, PeerEnv, PeerMessage, PotatoHandler, ToLocalUI,
    WalletSpendInterface,
};

#[allow(dead_code)]
enum NotificationToLocalUI {
    OpponentMoved(GameID, ReadableMove),
    MessageFromOpponent(GameID, ReadableMove),
    GameFinished(GameID, Amount),
    GameCancelled(GameID),
    ShutdownComplete(CoinString),
    GoingOnChain,
}

#[allow(dead_code)]
enum WalletBootstrapState {
    PartlySigned(Spend),
    FullySigned(Spend),
}

#[derive(Default)]
struct Pipe {
    // PacketSender
    queue: VecDeque<Vec<u8>>,

    // WalletSpendInterface
    outgoing_transactions: VecDeque<Spend>,
    registered_coins: HashMap<CoinID, Timeout>,

    // Game UI
    #[allow(dead_code)]
    game_starts: VecDeque<NotificationToLocalUI>,

    // Bootstrap info
    channel_puzzle_hash: Option<PuzzleHash>,

    #[allow(dead_code)]
    bootstrap_state: Option<WalletBootstrapState>,
}

impl PacketSender for Pipe {
    fn send_message(&mut self, msg: &PeerMessage) -> Result<(), Error> {
        let bson_doc = bson::to_bson(&msg).map_err(|e| Error::StrErr(format!("{e:?}")))?;
        let msg_data = bson::to_vec(&bson_doc).map_err(|e| Error::StrErr(format!("{e:?}")))?;
        self.queue.push_back(msg_data);
        Ok(())
    }
}

impl WalletSpendInterface for Pipe {
    fn spend_transaction_and_add_fee(&mut self, bundle: &Spend) -> Result<(), Error> {
        self.outgoing_transactions.push_back(bundle.clone());

        Ok(())
    }

    fn register_coin(&mut self, coin_id: &CoinID, timeout: &Timeout) -> Result<(), Error> {
        self.registered_coins
            .insert(coin_id.clone(), timeout.clone());

        Ok(())
    }
}

impl BootstrapTowardWallet for Pipe {
    fn channel_puzzle_hash(&mut self, puzzle_hash: &PuzzleHash) -> Result<(), Error> {
        self.channel_puzzle_hash = Some(puzzle_hash.clone());
        Ok(())
    }

    fn received_channel_offer(&mut self, _bundle: &SpendBundle) -> Result<(), Error> {
        todo!();
    }

    fn received_channel_transaction_completion(
        &mut self,
        _bundle: &SpendBundle,
    ) -> Result<(), Error> {
        todo!();
    }
}

impl ToLocalUI for Pipe {
    fn opponent_moved(&mut self, _id: &GameID, _readable: ReadableMove) -> Result<(), Error> {
        todo!();
    }
    fn game_message(&mut self, _id: &GameID, _readable: ReadableMove) -> Result<(), Error> {
        todo!();
    }
    fn game_finished(&mut self, _id: &GameID, _my_share: Amount) -> Result<(), Error> {
        todo!();
    }
    fn game_cancelled(&mut self, _id: &GameID) -> Result<(), Error> {
        todo!();
    }

    fn shutdown_complete(&mut self, _reward_coin_string: &CoinString) -> Result<(), Error> {
        todo!();
    }
    fn going_on_chain(&mut self) -> Result<(), Error> {
        todo!();
    }
}

fn run_move<'a, R: Rng>(
    env: &'a mut ChannelHandlerEnv<'a, R>,
    amount: Amount,
    pipe: &'a mut [Pipe; 2],
    peer: &mut PotatoHandler,
    who: usize,
) -> Result<(), Error> {
    let msg = pipe[who ^ 1].queue.pop_front().unwrap();

    let mut penv: PeerEnv<Pipe, R> = PeerEnv {
        env: env,
        system_interface: &mut pipe[who],
    };
    peer.received_message(&mut penv, msg)?;

    let parent = CoinString::from_parts(&CoinID::default(), &PuzzleHash::default(), &amount);

    if let Some(ch) = penv.system_interface.channel_puzzle_hash.clone() {
        penv.test_handle_received_channel_puzzle_hash(peer, &parent, &ch)?;
    }

    Ok(())
}

#[test]
fn test_peer_smoke() {
    let seed: [u8; 32] = [0; 32];
    let mut rng = ChaCha8Rng::from_seed(seed);
    let mut allocator = AllocEncoder::new();

    let mut pipe_sender: [Pipe; 2] = Default::default();

    let new_peer = |allocator: &mut AllocEncoder, rng: &mut ChaCha8Rng, have_potato: bool| {
        let private_keys1: ChannelHandlerPrivateKeys = rng.gen();
        let reward_private_key1: PrivateKey = rng.gen();
        let reward_public_key1 = private_to_public_key(&reward_private_key1);
        let reward_puzzle_hash1 =
            puzzle_hash_for_pk(allocator, &reward_public_key1).expect("should work");

        PotatoHandler::new(
            have_potato,
            private_keys1,
            Amount::new(100),
            Amount::new(100),
            reward_puzzle_hash1.clone(),
        )
    };

    let parent_private_key: PrivateKey = rng.gen();
    let parent_public_key = private_to_public_key(&parent_private_key);
    let parent_puzzle_hash =
        puzzle_hash_for_pk(&mut allocator, &parent_public_key).expect("should work");

    let parent_coin_id = CoinID::default();
    let parent_coin =
        CoinString::from_parts(&parent_coin_id, &parent_puzzle_hash, &Amount::new(200));

    let p1 = new_peer(&mut allocator, &mut rng, true);
    let p2 = new_peer(&mut allocator, &mut rng, false);
    let mut peers = [p1, p2];

    {
        let mut env = channel_handler_env(&mut allocator, &mut rng);
        let mut penv = PeerEnv {
            env: &mut env,
            system_interface: &mut pipe_sender[0],
        };
        peers[0].start(&mut penv, parent_coin).expect("should work");
    };

    // XXX Keep going to more message handling.
    for i in 1..=3 {
        let mut env = channel_handler_env(&mut allocator, &mut rng);
        let who = i % 2;
        run_move(
            &mut env,
            Amount::new(200),
            &mut pipe_sender,
            &mut peers[who],
            who,
        )
        .expect("should send");
    }
}