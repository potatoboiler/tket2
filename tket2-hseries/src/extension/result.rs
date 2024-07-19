//! This module defines the Hugr extension used to represent result reporting operations,
//! with static string tags.
//!
use hugr::types::Signature;
use hugr::{
    builder::{BuildError, Dataflow},
    extension::{
        prelude::{self, BOOL_T, PRELUDE, STRING_CUSTOM_TYPE},
        simple_op::{try_from_name, MakeExtensionOp, MakeOpDef, MakeRegisteredOp, OpLoadError},
        ExtensionId, ExtensionRegistry, ExtensionSet, OpDef, SignatureFunc,
    },
    ops::{CustomOp, NamedOp, OpType},
    std_extensions::arithmetic::{
        float_types::{
            EXTENSION as FLOAT_EXTENSION, EXTENSION_ID as FLOAT_EXTENSION_ID, FLOAT64_TYPE,
        },
        int_types::{
            int_type, EXTENSION as INT_EXTENSION, EXTENSION_ID as INT_EXTENSION_ID,
            LOG_WIDTH_TYPE_PARAM,
        },
    },
    type_row,
    types::{
        type_param::{CustomTypeArg, TypeParam},
        PolyFuncType, Type, TypeArg,
    },
    Extension, Wire,
};

use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use strum_macros::{EnumIter, EnumString, IntoStaticStr};

/// The "tket2.result" extension id.
pub const EXTENSION_ID: ExtensionId = ExtensionId::new_unchecked("tket2.result");

lazy_static! {
    /// The "tket2.result" extension.
    pub static ref EXTENSION: Extension = {
        let mut ext = Extension::new_with_reqs(EXTENSION_ID, ExtensionSet::from_iter([INT_EXTENSION_ID, FLOAT_EXTENSION_ID]));
        ResultOpDef::load_all_ops(&mut ext).unwrap();
        ext
    };

    /// Extension registry including the "tket2.result" extension and
    /// dependencies.
    pub static ref REGISTRY: ExtensionRegistry = ExtensionRegistry::try_new([
        EXTENSION.to_owned(),
        INT_EXTENSION.to_owned(),
        FLOAT_EXTENSION.to_owned(),
        PRELUDE.to_owned()
    ]).unwrap();
}

#[derive(
    Clone,
    Copy,
    Debug,
    Serialize,
    Deserialize,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    EnumIter,
    IntoStaticStr,
    EnumString,
)]
#[allow(missing_docs)]
#[non_exhaustive]
/// Result report operations from quantum runtime.
/*
result_int<Tag: StringArg, N: BoundedNat>( int<N> ) // N is bitwidth, e.g. i32, i64
result_uint<Tag: StringArg, N: BoundedNat>( int<N> ) // unsigned
result_bool<Tag: StringArg>( Sum((), ()) )
result_f64<Tag: StringArg>( f64 )

result_arr_int<Tag: StringArg, N: Nat, M: BoundedNat>( Array<N, int<M> > )
result_arr_uint<Tag: StringArg, N: Nat, M: BoundedNat>( Array<N, int<M> > )
result_arr_f64<Tag: StringArg, N: Nat>( Array<N,f64> )
result_arr_bool<Tag: StringArg, N: Nat>( Array<N, Sum((), ()) > )
*/
pub enum ResultOpDef {
    #[strum(serialize = "result_bool")]
    Bool,
    #[strum(serialize = "result_int")]
    Int,
    #[strum(serialize = "result_uint")]
    UInt,
    #[strum(serialize = "result_f64")]
    F64,
    #[strum(serialize = "result_array_bool")]
    ArrBool,
    #[strum(serialize = "result_array_int")]
    ArrInt,
    #[strum(serialize = "result_array_uint")]
    ArrUInt,
    #[strum(serialize = "result_array_f64")]
    ArrF64,
}

impl ResultOpDef {
    fn arg_type(&self) -> Type {
        match self {
            Self::Bool => BOOL_T,
            Self::Int | Self::UInt => int_tv(1),
            Self::F64 => FLOAT64_TYPE,
            Self::ArrBool | Self::ArrF64 => {
                let inner_t = self.simple_type_op().arg_type();
                array_type(inner_t)
            }
            Self::ArrInt | Self::ArrUInt => array_type(int_tv(2)),
        }
    }

    fn simple_type_op(&self) -> Self {
        match self {
            Self::ArrBool => Self::Bool,
            Self::ArrInt => Self::Int,
            Self::ArrUInt => Self::UInt,
            Self::ArrF64 => Self::F64,
            _ => *self,
        }
    }

    fn array_type_op(&self) -> Self {
        match self {
            Self::Bool => Self::ArrBool,
            Self::Int => Self::ArrInt,
            Self::UInt => Self::ArrUInt,
            Self::F64 => Self::ArrF64,
            _ => *self,
        }
    }

    fn type_params(&self) -> Vec<TypeParam> {
        match self {
            Self::Bool | Self::F64 => vec![],
            Self::Int | Self::UInt => vec![LOG_WIDTH_TYPE_PARAM],
            _ => [
                vec![TypeParam::max_nat()],
                self.simple_type_op().type_params(),
            ]
            .concat(),
        }
    }

    fn instantiate(&self, args: &[TypeArg]) -> Result<ResultOp, OpLoadError> {
        let parsed_args = concrete_result_op_type_args(args)?;

        match (parsed_args, self) {
            ((tag, None, None), Self::Bool | Self::F64) => Ok(ResultOp::_new_basic(tag, *self)),
            ((tag, Some(width), None), Self::Int | Self::UInt) => {
                Ok(ResultOp::_new_int(tag, width as u8, *self))
            }
            ((_, Some(size), _), _) => {
                let inner_args = match args {
                    [t, _] => vec![t.clone()],
                    [t, _, w] => vec![t.clone(), w.clone()],
                    _ => unreachable!(),
                };
                Ok(self
                    .simple_type_op()
                    .instantiate(&inner_args)?
                    .array_op(size))
            }
            _ => Err(hugr::extension::SignatureError::InvalidTypeArgs.into()),
        }
    }

    fn result_signature(&self) -> SignatureFunc {
        let string_param = TypeParam::Opaque {
            ty: STRING_CUSTOM_TYPE,
        };

        PolyFuncType::new(
            [vec![string_param], self.type_params()].concat(),
            Signature::new(self.arg_type(), type_row![]),
        )
        .into()
    }
}

fn array_type(inner_t: Type) -> Type {
    prelude::array_type(TypeArg::new_var_use(1, TypeParam::max_nat()), inner_t)
}

fn int_tv(int_tv_idx: usize) -> Type {
    int_type(TypeArg::new_var_use(int_tv_idx, LOG_WIDTH_TYPE_PARAM))
}

impl MakeOpDef for ResultOpDef {
    fn signature(&self) -> SignatureFunc {
        self.result_signature()
    }

    fn from_def(op_def: &OpDef) -> Result<Self, hugr::extension::simple_op::OpLoadError> {
        try_from_name(op_def.name(), &EXTENSION_ID)
    }

    fn extension(&self) -> ExtensionId {
        EXTENSION_ID
    }

    fn description(&self) -> String {
        match self {
            Self::Bool => "Report a boolean result.",
            Self::Int => "Report a signed integer result.",
            Self::UInt => "Report an unsigned integer result.",
            Self::F64 => "Report a floating-point result.",
            Self::ArrBool => "Report an array of boolean results.",
            Self::ArrInt => "Report an array of signed integer results.",
            Self::ArrUInt => "Report an array of unsigned integer results.",
            Self::ArrF64 => "Report an array of floating-point results.",
        }
        .to_string()
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Hash, PartialEq)]
enum SimpleArgs {
    Basic,
    Int(u8),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Hash, PartialEq)]
enum ResultArgs {
    Simple(SimpleArgs),
    Array(SimpleArgs, u64),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Hash, PartialEq)]
/// Concrete instantiation of a "tket2.result" operation.
pub struct ResultOp {
    tag: String,
    result_op: ResultOpDef,
    args: ResultArgs,
}

impl ResultOp {
    fn _new_basic(tag: impl Into<String>, result_op: ResultOpDef) -> Self {
        Self {
            tag: tag.into(),
            result_op,
            args: ResultArgs::Simple(SimpleArgs::Basic),
        }
    }

    fn _new_int(tag: impl Into<String>, int_width: u8, int_op: ResultOpDef) -> Self {
        Self {
            tag: tag.into(),
            result_op: int_op,
            args: ResultArgs::Simple(SimpleArgs::Int(int_width)),
        }
    }
    /// Create a new "tket2.result" operation for a boolean result.
    pub fn new_bool(tag: impl Into<String>) -> Self {
        Self::_new_basic(tag, ResultOpDef::Bool)
    }

    /// Create a new "tket2.result" operation for a floating-point result.
    pub fn new_f64(tag: impl Into<String>) -> Self {
        Self::_new_basic(tag, ResultOpDef::F64)
    }

    /// Convert this "tket2.result" operation to an array result operation over the same inner type.
    /// The size of the array is set to the given value.
    /// If this operation is already an array result operation, its size is updated.
    pub fn array_op(mut self, size: u64) -> Self {
        let result_op = self.result_op.array_type_op();
        match &mut self.args {
            ResultArgs::Simple(s_args) => {
                self.args = ResultArgs::Array(s_args.clone(), size);
                self.result_op = result_op;
                self
            }
            ResultArgs::Array(_, s) => {
                *s = size;
                self
            }
        }
    }

    /// Create a new "tket2.result" operation for a signed integer result of a given bit width.
    pub fn new_int(tag: impl Into<String>, int_width: u8) -> Self {
        Self::_new_int(tag, int_width, ResultOpDef::Int)
    }

    /// Create a new "tket2.result" operation for an unsigned integer result of a given bit width.
    pub fn new_uint(tag: impl Into<String>, int_width: u8) -> Self {
        Self::_new_int(tag, int_width, ResultOpDef::UInt)
    }
}

fn concrete_result_op_type_args(
    args: &[TypeArg],
) -> Result<(String, Option<u64>, Option<u64>), OpLoadError> {
    let err = || hugr::extension::SignatureError::InvalidTypeArgs.into();
    let extract_string =
        |arg: &CustomTypeArg| arg.value.as_str().map(|s| s.to_string()).ok_or(err());
    match args {
        [TypeArg::Opaque { arg }] => Ok((extract_string(arg)?, None, None)),

        [TypeArg::Opaque { arg }, TypeArg::BoundedNat { n }] => {
            Ok((extract_string(arg)?, Some(*n), None))
        }

        [TypeArg::Opaque { arg }, TypeArg::BoundedNat { n }, TypeArg::BoundedNat { n: m }] => {
            Ok((extract_string(arg)?, Some(*n), Some(*m)))
        }

        _ => Err(err()),
    }
}

impl<'a> From<&'a ResultOp> for &'static str {
    fn from(value: &ResultOp) -> Self {
        value.result_op.into()
    }
}

impl MakeExtensionOp for ResultOp {
    fn from_extension_op(
        ext_op: &hugr::ops::custom::ExtensionOp,
    ) -> Result<Self, hugr::extension::simple_op::OpLoadError>
    where
        Self: Sized,
    {
        let def = ext_op.def();
        let args = ext_op.args();
        ResultOpDef::from_def(def)?.instantiate(args)
    }

    fn type_args(&self) -> Vec<TypeArg> {
        let mut type_args = vec![TypeArg::Opaque {
            arg: CustomTypeArg::new(STRING_CUSTOM_TYPE, self.tag.clone().into()).unwrap(),
        }];

        match self.args {
            ResultArgs::Simple(_) => {}
            ResultArgs::Array(_, size) => {
                type_args.push(TypeArg::BoundedNat { n: size });
            }
        }

        match self.args {
            ResultArgs::Simple(SimpleArgs::Int(width))
            | ResultArgs::Array(SimpleArgs::Int(width), _) => {
                type_args.push(TypeArg::BoundedNat { n: width as u64 });
            }
            _ => {}
        }

        type_args
    }
}

impl MakeRegisteredOp for ResultOp {
    fn extension_id(&self) -> ExtensionId {
        EXTENSION_ID
    }

    fn registry<'s, 'r: 's>(&'s self) -> &'r ExtensionRegistry {
        &REGISTRY
    }
}

impl TryFrom<&OpType> for ResultOpDef {
    type Error = ();

    fn try_from(value: &OpType) -> Result<Self, Self::Error> {
        let Some(custom_op) = value.as_custom_op() else {
            Err(())?
        };
        match custom_op {
            CustomOp::Extension(ext) => Self::from_extension_op(ext).ok(),
            CustomOp::Opaque(opaque) => try_from_name(opaque.name(), &EXTENSION_ID).ok(),
        }
        .ok_or(())
    }
}

impl TryFrom<&OpType> for ResultOp {
    type Error = OpLoadError;

    fn try_from(value: &OpType) -> Result<Self, Self::Error> {
        let Some(custom_op) = value.as_custom_op() else {
            Err(OpLoadError::NotMember(value.name().into()))?
        };
        match custom_op {
            CustomOp::Extension(ext) => Self::from_extension_op(ext),
            CustomOp::Opaque(opaque) => try_from_name::<ResultOpDef>(opaque.name(), &EXTENSION_ID)?
                .instantiate(opaque.args()),
        }
    }
}

/// An extension trait for [Dataflow] providing methods to add "tket2.result"
/// operations.
pub trait ResultOpBuilder: Dataflow {
    /// Add a "tket2.result" op.
    fn add_result(&mut self, result_wire: Wire, op: ResultOp) -> Result<(), BuildError> {
        let handle = self.add_dataflow_op(op, [result_wire])?;

        debug_assert_eq!(handle.outputs().len(), 0);
        Ok(())
    }
}

impl<D: Dataflow> ResultOpBuilder for D {}

#[cfg(test)]
pub(crate) mod test {
    use cool_asserts::assert_matches;
    use hugr::types::Signature;
    use hugr::{
        builder::{Dataflow, DataflowHugr, FunctionBuilder},
        extension::prelude::array_type,
        ops::NamedOp,
        std_extensions::arithmetic::int_types::INT_TYPES,
    };
    use std::sync::Arc;
    use strum::IntoEnumIterator;

    use super::*;

    fn get_opdef(op: impl NamedOp) -> Option<&'static Arc<OpDef>> {
        EXTENSION.get_op(&op.name())
    }

    #[test]
    fn create_extension() {
        assert_eq!(EXTENSION.name(), &EXTENSION_ID);

        for o in ResultOpDef::iter() {
            assert_eq!(ResultOpDef::from_def(get_opdef(o).unwrap()), Ok(o));
        }
    }

    #[test]
    fn circuit() {
        const ARR_SIZE: u64 = 20;
        let in_row = vec![
            BOOL_T,
            FLOAT64_TYPE,
            INT_TYPES[5].clone(),
            INT_TYPES[6].clone(),
        ];
        let in_row = [
            in_row.clone(),
            in_row
                .into_iter()
                .map(|t| array_type(TypeArg::BoundedNat { n: ARR_SIZE }, t))
                .collect(),
        ]
        .concat();
        let hugr = {
            let mut func_builder =
                FunctionBuilder::new("circuit", Signature::new(in_row, type_row![])).unwrap();
            let ops = [
                ResultOp::new_bool("b"),
                ResultOp::new_f64("f"),
                ResultOp::new_int("i", 5),
                ResultOp::new_uint("u", 6),
            ];

            for op in &ops {
                let op_t: OpType = op.clone().to_extension_op().unwrap().into();
                let def_op: ResultOpDef = (&op_t).try_into().unwrap();
                assert_eq!(op.result_op, def_op);
                let new_op: ResultOp = (&op_t).try_into().unwrap();
                assert_eq!(&new_op, op);

                let op = op.clone().array_op(ARR_SIZE);
                let op_t: OpType = op.clone().to_extension_op().unwrap().into();
                let def_op: ResultOpDef = (&op_t).try_into().unwrap();

                assert_eq!(op.result_op, def_op);
                let new_op: ResultOp = (&op_t).try_into().unwrap();
                assert_eq!(&new_op, &op);
            }
            let [b, f, i, u, a_b, a_f, a_i, a_u] = func_builder.input_wires_arr();

            for (w, op) in [b, f, i, u].iter().zip(ops.iter()) {
                func_builder.add_result(*w, op.clone()).unwrap();
            }
            for (w, op) in [a_b, a_f, a_i, a_u].iter().zip(ops.iter()) {
                func_builder
                    .add_result(*w, op.clone().array_op(ARR_SIZE))
                    .unwrap();
            }

            func_builder
                .finish_hugr_with_outputs([], &REGISTRY)
                .unwrap()
        };
        assert_matches!(hugr.validate(&REGISTRY), Ok(_));
    }
}
