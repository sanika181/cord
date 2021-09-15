// Copyright 2019-2021 Dhiway.
// This file is part of CORD Platform.

#![cfg_attr(not(feature = "std"), no_std)]
#![allow(clippy::unused_unit)]

use cord_primitives::{IdentifierOf, StatusOf};
use frame_support::{ensure, storage::types::StorageMap};
use sp_std::{fmt::Debug, prelude::Clone, str, vec::Vec};

pub mod streams;
pub mod weights;

pub use crate::streams::*;

use crate::weights::WeightInfo;
pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;

	/// ID of an entity.
	pub type IdOf<T> = <T as frame_system::Config>::Hash;
	/// Hash of the transaction.
	pub type HashOf<T> = <T as frame_system::Config>::Hash;
	/// Type of a entity controller.
	pub type CordAccountOf<T> = pallet_schema::CordAccountOf<T>;
	/// Type for a block number.
	pub type BlockNumberOf<T> = <T as frame_system::Config>::BlockNumber;

	#[pallet::config]
	pub trait Config: frame_system::Config + pallet_schema::Config {
		type EnsureOrigin: EnsureOrigin<
			Success = CordAccountOf<Self>,
			<Self as frame_system::Config>::Origin,
		>;
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;
		type WeightInfo: WeightInfo;
	}

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {}

	/// streams stored on chain.
	/// It maps from stream Id to its details.
	#[pallet::storage]
	#[pallet::getter(fn streams)]
	pub type Streams<T> = StorageMap<_, Blake2_128Concat, IdOf<T>, StreamDetails<T>>;

	/// stream commit details stored on chain.
	/// It maps from a stream Id to a vector of commit details.
	#[pallet::storage]
	#[pallet::getter(fn commits)]
	pub type Commits<T> = StorageMap<_, Blake2_128Concat, IdOf<T>, Vec<StreamCommit<T>>>;

	/// stream links stored on chain.
	/// It maps from a stream Id to a vector of links.
	#[pallet::storage]
	#[pallet::getter(fn links)]
	pub type Links<T> = StorageMap<_, Blake2_128Concat, IdOf<T>, Vec<StreamLink<T>>>;

	/// stream hashes stored on chain.
	/// It maps from a stream hash to Id.
	#[pallet::storage]
	#[pallet::getter(fn hashes)]
	pub type Hashes<T> = StorageMap<_, Blake2_128Concat, HashOf<T>, IdOf<T>>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// A new entity has been created.
		/// \[entity identifier, controller\]
		TxAdd(IdOf<T>, HashOf<T>, CordAccountOf<T>),
		/// An entityhas been created.
		/// \[entity identifier, controller\]
		TxUpdate(IdOf<T>, HashOf<T>, CordAccountOf<T>),
		/// An entity has been revoked.
		/// \[entity identifier\]
		TxStatus(IdOf<T>, CordAccountOf<T>),
	}

	#[pallet::error]
	pub enum Error<T> {
		/// Invalid request
		InvalidRequest,
		/// Hash and ID are the same
		SameIdentifierAndHash,
		/// Transaction idenfier is not unique
		StreamAlreadyAnchored,
		/// Transaction idenfier not found
		StreamNotFound,
		/// Transaction idenfier marked inactive
		StreamRevoked,
		/// Invalid CID encoding.
		InvalidCidEncoding,
		/// CID already anchored
		CidAlreadyAnchored,
		/// no status change required
		StatusChangeNotRequired,
		/// Only when the author is not the controller.
		UnauthorizedOperation,
		/// Stream link does not exist
		StreamLinkNotFound,
		/// Linked stream is revoked
		StreamLinkRevoked,
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Create a new stream and associates it with its controller.
		///
		/// * origin: the identifier of the stream controller
		/// * identifier: unique identifier of the incoming stream.
		/// * hash: hash of the incoming stream.
		/// * cid: SID of the incoming  stream.
		/// * schema: stream schema.
		/// * link: stream link.
		#[pallet::weight(0)]
		pub fn create(
			origin: OriginFor<T>,
			identifier: IdOf<T>,
			hash: HashOf<T>,
			cid: Option<IdentifierOf>,
			schema: Option<IdOf<T>>,
			link: Option<IdOf<T>>,
		) -> DispatchResult {
			let controller = <T as Config>::EnsureOrigin::ensure_origin(origin)?;
			ensure!(hash != identifier, Error::<T>::SameIdentifierAndHash);
			//check store Id encoding
			if let Some(ref cid) = cid {
				pallet_schema::SchemaDetails::<T>::is_valid(cid)?;
			}
			ensure!(!<Streams<T>>::contains_key(&identifier), Error::<T>::StreamAlreadyAnchored);
			//check stream schema status
			if let Some(schema) = schema {
				pallet_schema::SchemaDetails::<T>::schema_status(schema, controller.clone())
					.map_err(<pallet_schema::Error<T>>::from)?;
			}
			//check link status
			if let Some(ref link) = link {
				let links = <Streams<T>>::get(&link).ok_or(Error::<T>::StreamLinkNotFound)?;
				ensure!(!links.revoked, Error::<T>::StreamLinkRevoked);
				StreamLink::<T>::link_tx(
					&link,
					StreamLink { identifier: identifier.clone(), controller: controller.clone() },
				)?;
			}

			let block_number = <frame_system::Pallet<T>>::block_number();

			StreamCommit::<T>::store_tx(
				&identifier,
				StreamCommit {
					hash: hash.clone(),
					cid: cid.clone(),
					block: block_number.clone(),
					commit: StreamCommitOf::Genesis,
				},
			)?;

			<Hashes<T>>::insert(&hash, &identifier);

			<Streams<T>>::insert(
				&identifier,
				StreamDetails {
					hash: hash.clone(),
					cid,
					parent_cid: None,
					schema,
					link,
					controller: controller.clone(),
					block: block_number,
					revoked: false,
				},
			);
			Self::deposit_event(Event::TxAdd(identifier, hash, controller));

			Ok(())
		}
		/// Updates the stream information.
		///
		/// * origin: the identifier of the stream controller
		/// * identifier: unique identifier of the incoming stream.
		/// * hash: hash of the incoming stream.
		/// * cid: storage Id of the incoming stream.
		#[pallet::weight(0)]
		pub fn update(
			origin: OriginFor<T>,
			identifier: IdOf<T>,
			hash: HashOf<T>,
			cid: Option<IdentifierOf>,
		) -> DispatchResult {
			let updater = <T as Config>::EnsureOrigin::ensure_origin(origin)?;
			ensure!(hash != identifier, Error::<T>::SameIdentifierAndHash);

			let tx_prev = <Streams<T>>::get(&identifier).ok_or(Error::<T>::StreamNotFound)?;
			//check cid encoding
			if let Some(ref cid) = cid {
				ensure!(cid != tx_prev.cid.as_ref().unwrap(), Error::<T>::CidAlreadyAnchored);
				pallet_schema::SchemaDetails::<T>::is_valid(cid)?;
			}
			ensure!(!tx_prev.revoked, Error::<T>::StreamRevoked);
			ensure!(tx_prev.controller == updater, Error::<T>::UnauthorizedOperation);

			let block_number = <frame_system::Pallet<T>>::block_number();

			StreamCommit::<T>::store_tx(
				&identifier,
				StreamCommit {
					hash: hash.clone(),
					cid: cid.clone(),
					block: block_number.clone(),
					commit: StreamCommitOf::Update,
				},
			)?;

			<Hashes<T>>::insert(&hash, &identifier);

			<Streams<T>>::insert(
				&identifier,
				StreamDetails {
					hash: hash.clone(),
					cid,
					parent_cid: tx_prev.cid,
					controller: updater.clone(),
					block: block_number,
					..tx_prev
				},
			);

			Self::deposit_event(Event::TxUpdate(identifier, hash, updater));

			Ok(())
		}
		/// Update the status of the stream
		///
		/// * origin: the identifier of the stream controller
		/// * identifier: unique identifier of the stream.
		/// * status: stream revocation status (bool).
		#[pallet::weight(0)]
		pub fn set_status(
			origin: OriginFor<T>,
			identifier: IdOf<T>,
			status: StatusOf,
		) -> DispatchResult {
			let updater = <T as Config>::EnsureOrigin::ensure_origin(origin)?;

			let tx_status = <Streams<T>>::get(&identifier).ok_or(Error::<T>::StreamNotFound)?;
			ensure!(tx_status.revoked != status, Error::<T>::StatusChangeNotRequired);
			ensure!(tx_status.controller == updater, Error::<T>::UnauthorizedOperation);

			let block_number = <frame_system::Pallet<T>>::block_number();

			StreamCommit::<T>::store_tx(
				&identifier,
				StreamCommit {
					hash: tx_status.hash.clone(),
					cid: tx_status.cid.clone(),
					block: block_number.clone(),
					commit: StreamCommitOf::StatusChange,
				},
			)?;

			<Streams<T>>::insert(
				&identifier,
				StreamDetails { block: block_number, revoked: status, ..tx_status },
			);

			Self::deposit_event(Event::TxStatus(identifier, updater));

			Ok(())
		}
	}
}