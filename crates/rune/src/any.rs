use core::any;

use crate::compile::Named;
use crate::runtime::{AnyTypeInfo, TypeHash};

/// The trait implemented for types which can be used inside of Rune.
///
/// This can only be implemented correctly through the [`Any`] derive.
/// Implementing it manually is not supported.
///
/// Rune only supports two types, *built-in* types like [`i64`] and *external*
/// types which derive `Any`. Before they can be used they must be registered in
/// [`Context::install`] through a [`Module`].
///
/// This is typically used in combination with declarative macros to register
/// functions and macros, such as [`rune::function`].
///
/// [`AnyObj`]: crate::runtime::AnyObj
/// [`Context::install`]: crate::Context::install
/// [`Module`]: crate::Module
/// [`String`]: std::string::String
/// [`rune::function`]: macro@crate::function
/// [`rune::macro_`]: macro@crate::macro_
/// [`Any`]: derive@crate::Any
///
/// # Examples
///
/// ```
/// use rune::Any;
///
/// #[derive(Any)]
/// struct Npc {
///     #[rune(get)]
///     health: u32,
/// }
///
/// impl Npc {
///     /// Construct a new NPC.
///     #[rune::function(path = Self::new)]
///     fn new(health: u32) -> Self {
///         Self {
///             health
///         }
///     }
///
///     /// Damage the NPC with the given `amount`.
///     #[rune::function]
///     fn damage(&mut self, amount: u32) {
///         self.health -= amount;
///     }
/// }
///
/// fn install() -> Result<rune::Module, rune::ContextError> {
///     let mut module = rune::Module::new();
///     module.ty::<Npc>()?;
///     module.function_meta(Npc::new)?;
///     module.function_meta(Npc::damage)?;
///     Ok(module)
/// }
/// ```
pub trait Any: TypeHash + Named + any::Any {
    /// The compile-time type information know for the type.
    const ANY_TYPE_INFO: AnyTypeInfo = AnyTypeInfo::new(Self::full_name, Self::HASH);
}

/// Trait implemented for types which can be automatically converted to a
/// [`Value`].
///
/// We can't use a blanked implementation over `T: Any` because it only governs
/// what can be stored in any [`AnyObj`].
///
/// This trait in contrast is selectively implemented for types which we want to
/// generate [`ToValue`] and [`FromValue`] implementations for.
///
/// [`Value`]: crate::runtime::Value
/// [`AnyObj`]: crate::runtime::AnyObj
/// [`ToValue`]: crate::runtime::ToValue
/// [`FromValue`]: crate::runtime::FromValue
///
/// Note that you are *not* supposed to implement this directly. Make use of the
/// [`Any`] derive instead.
///
/// [`Any`]: derive@crate::Any
pub trait AnyMarker: Any {}

/// Macro to mark a value as external, which will implement all the appropriate
/// traits.
///
/// This is required to support the external type as a type argument in a
/// registered function.
///
/// <br>
///
/// ## Container attributes
///
/// <br>
///
/// ### `#[rune(item = <path>)]`
///
/// Specify the item prefix which contains this time.
///
/// This is required in order to calculate the correct type hash, if this is
/// omitted and the item is defined in a nested module the type hash won't match
/// the expected path hash.
///
/// ```
/// use rune::{Any, Module};
///
/// #[derive(Any)]
/// #[rune(item = ::process)]
/// struct Process {
///     /* .. */
/// }
///
/// let mut m = Module::with_crate("process")?;
/// m.ty::<Process>()?;
/// # Ok::<_, rune::ContextError>(())
/// ```
///
/// <br>
///
/// ### `#[rune(name = <ident>)]` attribute
///
/// The name of a type defaults to its identifiers, so `struct Foo {}` would be
/// given the name `Foo`.
///
/// This can be overrided with the `#[rune(name = <ident>)]` attribute:
///
/// ```
/// use rune::{Any, Module};
///
/// #[derive(Any)]
/// #[rune(name = Bar)]
/// struct Foo {
/// }
///
/// let mut m = Module::new();
/// m.ty::<Foo>()?;
/// # Ok::<_, rune::ContextError>(())
/// ```
///
/// <br>
///
/// ### `#[rune(empty)]`, `#[rune(unnamed(<int>))]`
///
/// This attribute controls how the metadata of fields are handled in the type.
///
/// By default fields are registered depending on the type of structure or enum
/// being registered. This prevents the metadata from being further customized
/// through methods such as [`TypeMut::make_empty_struct`] since that would
/// result in duplicate metadata being registered.
///
/// To avoid this behavior, the `#[rune(fields)]` attribute can be used which
/// suppressed any field metadata from being generated for `none` or customized
/// like `empty`. If set to `none` then it leaves the field metadata free to be
/// configured manually during [`Module::ty`] setup.
///
/// Registering a type like this allows it to be used like an empty struct like
/// `let v = Struct;` despite having fields:
///
/// ```
/// use rune::{Any, Module};
///
/// #[derive(Any)]
/// #[rune(empty, constructor = Struct::new)]
/// struct Struct {
///     field: u32,
/// }
///
/// impl Struct {
///     fn new() -> Self {
///         Self { field: 42 }
///     }
/// }
///
/// let mut m = Module::new();
/// m.ty::<Struct>()?;
/// # Ok::<_, rune::ContextError>(())
/// ```
///
/// Support for an unnamed struct:
///
/// ```
/// use rune::{Any, Module};
///
/// #[derive(Any)]
/// #[rune(unnamed(2), constructor = Struct::new)]
/// struct Struct {
///     a: u32,
///     b: u32,
/// }
///
/// impl Struct {
///     fn new(a: u32, b: u32) -> Self {
///         Self { a, b }
///     }
/// }
///
/// let mut m = Module::new();
/// m.ty::<Struct>()?;
/// # Ok::<_, rune::ContextError>(())
/// ```
///
///
/// <br>
///
/// ### `#[rune(constructor)]`
///
/// This allows for specifying that a type has a rune-visible constructor, and
/// which method should be called to construct the value.
///
/// A constructor in this instance means supporting expressions such as:
///
/// * `Struct { field: 42 }` for named structs.
/// * `Struct(42)` for unnamed structs.
/// * `Struct` for empty structs.
///
/// By default the attribute will generate a constructor out of every field
/// which is marked with `#[rune(get)]`. The remaining fields must then
/// implement [`Default`].
///
/// ```
/// use rune::{Any, Module};
///
/// #[derive(Any)]
/// #[rune(constructor)]
/// struct Struct {
///     #[rune(get)]
///     a: u32,
///     b: u32,
/// }
///
/// let mut m = Module::new();
/// m.ty::<Struct>()?;
/// # Ok::<_, rune::ContextError>(())
/// ```
///
/// For fine-grained control over the constructor, `#[rune(constructor =
/// <path>)]` can be used.
///
/// ```
/// use rune::{Any, Module};
///
/// #[derive(Any)]
/// #[rune(empty, constructor = Struct::new)]
/// struct Struct {
///     field: u32,
/// }
///
/// impl Struct {
///     fn new() -> Self {
///         Self { field: 42 }
///     }
/// }
///
/// let mut m = Module::new();
/// m.ty::<Struct>()?;
/// # Ok::<_, rune::ContextError>(())
/// ```
///
/// ## Field attributes
///
/// <br>
///
/// ### Field functions
///
/// Field functions are special operations which operate on fields. These are
/// distinct from associated functions, because they are invoked by using the
/// operation associated with the kind of the field function.
///
/// The most common forms of fields functions are *getters* and *setters*, which
/// are defined through the [`Protocol::GET`] and [`Protocol::SET`] protocols.
///
/// The `Any` derive can also generate default implementations of these through
/// various `#[rune(...)]` attributes:
///
/// ```rust
/// use rune::{Any, Module};
///
/// #[derive(Any)]
/// struct Struct {
///     #[rune(get, set, add_assign, copy)]
///     number: i64,
///     #[rune(get, set)]
///     string: String,
/// }
///
/// let mut m = Module::new();
/// m.ty::<Struct>()?;
/// # Ok::<_, rune::ContextError>(())
/// ```
///
/// Once registered, this allows `External` to be used like this in Rune:
///
/// ```rune
/// pub fn main(external) {
///     external.number = external.number + 1;
///     external.number += 1;
///     external.string = `${external.string} World`;
/// }
/// ```
///
/// The full list of available field functions and their corresponding
/// attributes are:
///
/// | Protocol | Attribute | |
/// |-|-|-|
/// | [`Protocol::GET`] | `#[rune(get)]` | For getters, like `external.field`. |
/// | [`Protocol::SET`] | `#[rune(set)]` | For setters, like `external.field = 42`. |
/// | [`Protocol::ADD_ASSIGN`] | `#[rune(add_assign)]` | The `+=` operation. |
/// | [`Protocol::SUB_ASSIGN`] | `#[rune(sub_assign)]` | The `-=` operation. |
/// | [`Protocol::MUL_ASSIGN`] | `#[rune(mul_assign)]` | The `*=` operation. |
/// | [`Protocol::DIV_ASSIGN`] | `#[rune(div_assign)]` | The `/=` operation. |
/// | [`Protocol::BIT_AND_ASSIGN`] | `#[rune(bit_and_assign)]` | The `&=` operation. |
/// | [`Protocol::BIT_OR_ASSIGN`] | `#[rune(bit_or_assign)]` | The bitwise or operation. |
/// | [`Protocol::BIT_XOR_ASSIGN`] | `#[rune(bit_xor_assign)]` | The `^=` operation. |
/// | [`Protocol::SHL_ASSIGN`] | `#[rune(shl_assign)]` | The `<<=` operation. |
/// | [`Protocol::SHR_ASSIGN`] | `#[rune(shr_assign)]` | The `>>=` operation. |
/// | [`Protocol::REM_ASSIGN`] | `#[rune(rem_assign)]` | The `%=` operation. |
///
/// The manual way to register these functions is to use the new
/// `Module::field_function` function. This clearly showcases that there's no
/// relationship between the field used and the function registered:
///
/// ```rust
/// use rune::{Any, Module};
/// use rune::runtime::Protocol;
///
/// #[derive(Any)]
/// struct External {
/// }
///
/// impl External {
///     fn field_get(&self) -> String {
///         String::from("Hello World")
///     }
/// }
///
/// let mut module = Module::new();
/// module.field_function(&Protocol::GET, "field", External::field_get)?;
/// # Ok::<_, rune::support::Error>(())
/// ```
///
/// Would allow for this in Rune:
///
/// ```rune
/// pub fn main(external) {
///     println!("{}", external.field);
/// }
/// ```
///
/// ### Customizing how fields are cloned with `#[rune(get)]`
///
/// In order to return a value through `#[rune(get)]`, the value has to be
/// cloned.
///
/// By default, this is done through the [`TryClone` trait], but its behavior
/// can be customized through the following attributes:
///
/// <br>
///
/// ### `#[rune(copy)]`
///
/// This indicates that the field is `Copy`.
///
/// <br>
///
/// ### `#[rune(clone)]`
///
/// This indicates that the field should use `std::clone::Clone` to clone the
/// value. Note that this effecitvely means that the memory the value uses
/// during cloning is *not* tracked and should be avoided in favor of using
/// [`rune::alloc`] and the [`TryClone` trait] without good reason.
///
/// <br>
///
/// ### `#[rune(clone_with = <path>)]`
///
/// This specified a custom method that should be used to clone the value.
///
/// ```rust
/// use rune::Any;
/// use rune::sync::Arc;
///
/// #[derive(Any)]
/// struct External {
///     #[rune(get, clone_with = Inner::clone)]
///     field: Inner,
/// }
///
/// #[derive(Any, Clone)]
/// struct Inner {
///     name: Arc<String>,
/// }
/// ```
///
/// <br>
///
/// ### `#[rune(try_clone_with = <path>)]`
///
/// This specified a custom method that should be used to clone the value.
///
/// ```rust
/// use rune::Any;
/// use rune::alloc::prelude::*;
///
/// #[derive(Any)]
/// struct External {
///     #[rune(get, try_clone_with = String::try_clone)]
///     field: String,
/// }
/// ```
///
/// [`Module::ty`]: crate::Module::ty
/// [`Protocol::ADD_ASSIGN`]: crate::runtime::Protocol::ADD_ASSIGN
/// [`Protocol::BIT_AND_ASSIGN`]: crate::runtime::Protocol::BIT_AND_ASSIGN
/// [`Protocol::BIT_OR_ASSIGN`]: crate::runtime::Protocol::BIT_OR_ASSIGN
/// [`Protocol::BIT_XOR_ASSIGN`]: crate::runtime::Protocol::BIT_XOR_ASSIGN
/// [`Protocol::DIV_ASSIGN`]: crate::runtime::Protocol::DIV_ASSIGN
/// [`Protocol::GET`]: crate::runtime::Protocol::GET
/// [`Protocol::MUL_ASSIGN`]: crate::runtime::Protocol::MUL_ASSIGN
/// [`Protocol::REM_ASSIGN`]: crate::runtime::Protocol::REM_ASSIGN
/// [`Protocol::SET`]: crate::runtime::Protocol::SET
/// [`Protocol::SHL_ASSIGN`]: crate::runtime::Protocol::SHL_ASSIGN
/// [`Protocol::SHR_ASSIGN`]: crate::runtime::Protocol::SHR_ASSIGN
/// [`Protocol::SUB_ASSIGN`]: crate::runtime::Protocol::SUB_ASSIGN
/// [`rune::alloc`]: crate::alloc
/// [`TryClone` trait]: crate::alloc::clone::TryClone
/// [`TypeMut::make_empty_struct`]: crate::module::TypeMut::make_empty_struct
pub use rune_macros::Any;
