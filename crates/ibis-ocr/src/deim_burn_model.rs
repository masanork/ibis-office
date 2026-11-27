// Generated from ONNX "/home/antigravity/.gemini/jetski/brain/v1/default/jetski_remote_agents_3p_devbox/6706d7dd-4907-4965-8a12-02bd466ffcea/scratch/ndlocr-lite/src/model/deim-s-1024x1024-261124.onnx" by burn-onnx
extern crate alloc;
use burn::prelude::*;
use burn::nn::LayerNorm;
use burn::nn::LayerNormConfig;
use burn::nn::Linear;
use burn::nn::LinearConfig;
use burn::nn::PaddingConfig2d;
use burn::nn::conv::Conv2d;
use burn::nn::conv::Conv2dConfig;
use burn::nn::pool::MaxPool2d;
use burn::nn::pool::MaxPool2dConfig;
use burn::tensor::Bytes;
use burn::tensor::TensorData;
use burn_store::BurnpackStore;
use burn_store::ModuleSnapshot;

/// Runtime allocation device, moved alongside the model's parameters.
#[derive(Clone, Debug)]
struct AllocationDevice(Device);
impl burn::module::Module for AllocationDevice {
    fn collect_devices(
        &self,
        mut devices: burn::module::Devices,
    ) -> burn::module::Devices {
        if !devices.contains(&self.0) {
            devices.push(self.0.clone());
        }
        devices
    }
    fn to_device(self, device: &Device) -> Self {
        Self(device.clone().with_autodiff_context_from(&self.0))
    }
    fn fork(self, device: &Device) -> Self {
        self.to_device(device)
    }
    fn visit<V: burn::module::ModuleVisitor>(&self, _visitor: &mut V) {}
    fn map<M: burn::module::ModuleMapper>(self, _mapper: &mut M) -> Self {
        self
    }
    fn materialize(self) -> Self {
        self
    }
    fn valid(&self) -> Self {
        Self(self.0.clone().without_autodiff())
    }
    fn train(self) -> Self {
        #[allow(dead_code)]
        trait AutodiffFallback {
            fn autodiff(self) -> Self;
        }
        impl AutodiffFallback for Device {
            fn autodiff(self) -> Self {
                self
            }
        }
        Self(self.0.autodiff())
    }
}
impl burn::module::ModuleDisplayDefault for AllocationDevice {
    fn content(&self, content: burn::module::Content) -> Option<burn::module::Content> {
        content.add_debug_attribute("device", &self.0).optional()
    }
}
impl burn::module::ModuleDisplay for AllocationDevice {}

#[derive(Module, Debug)]
pub struct Submodule1 {
    conv2d1: Conv2d,
    constant1: burn::module::Param<Tensor<3>>,
    constant429: burn::module::Param<Tensor<1>>,
    constant430: burn::module::Param<Tensor<1>>,
    constant432: burn::module::Param<Tensor<1>>,
    device: AllocationDevice,
}
impl Submodule1 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let conv2d1 = Conv2dConfig::new([3, 192], [16, 16])
            .with_stride([16, 16])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let constant1: burn::module::Param<Tensor<3>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                3,
            >::zeros([1, 1, 192], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1, 1, 192].into(),
        );
        let constant429: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant430: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant432: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([6.2831854820251465f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        Self {
            conv2d1,
            constant1,
            constant429,
            constant430,
            constant432,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `images`: expected dtype `DType::F32`
    pub fn forward(&self, images: Tensor<4>) -> (Tensor<3>, Tensor<3>, i64, i64, i64) {
        let shape1_out1: [i64; 4] = {
            let axes = &images.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather1_out1 = shape1_out1[2] as i64;
        let constant397_out1 = 16i64;
        let div1_out1 = gather1_out1 / constant397_out1;
        let gather2_out1 = shape1_out1[3] as i64;
        let constant400_out1 = 16i64;
        let div2_out1 = gather2_out1 / constant400_out1;
        let gather3_out1 = shape1_out1[0] as i64;
        let gather4_out1 = shape1_out1[2] as i64;
        let gather5_out1 = shape1_out1[3] as i64;
        let conv2d1_out1 = self.conv2d1.forward(images);
        let shape6_out1: [i64; 4] = {
            let axes = &conv2d1_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice1_out1: [i64; 2] = shape6_out1[0..2].try_into().unwrap();
        let constant410_out1: [i64; 1] = [-1i64];
        let concat1_out1: [i64; 3usize] = [&slice1_out1[..], &constant410_out1[..]]
            .concat()
            .try_into()
            .unwrap();
        let reshape1_out1 = conv2d1_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat1_out1));
        let transpose1_out1 = reshape1_out1.permute([0, 2, 1]);
        let shape7_out1: [i64; 3] = {
            let axes = &transpose1_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather6_out1 = shape7_out1[0] as i64;
        let unsqueeze1_out1 = [gather6_out1 as i64];
        let constant414_out1: [i64; 1] = [-1i64];
        let constant413_out1: [i64; 1] = [-1i64];
        let concat2_out1: [i64; 3usize] = [
            &unsqueeze1_out1[..],
            &constant413_out1[..],
            &constant414_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape2_out1 = concat2_out1;
        let mul1_out1: [i64; 3] = [-1i64, -1i64, -1i64];
        let equal1_out1 = {
            let __lhs = reshape2_out1;
            let __rhs = mul1_out1;
            core::array::from_fn::<
                i64,
                3usize,
                _,
            >(|__i| if __lhs[__i] == __rhs[__i] { 1i64 } else { 0i64 })
        };
        let constantofshape1_out1: [i64; 3] = [1i64, 1i64, 1i64];
        let where1_out1 = {
            let mut result = reshape2_out1;
            for (i, (cond_item, x_item)) in equal1_out1
                .iter()
                .zip(constantofshape1_out1.iter())
                .enumerate()
            {
                if *cond_item != 0 {
                    result[i] = *x_item;
                }
            }
            result
        };
        let constant1_out1 = self.constant1.val();
        let expand1_out1 = {
            let onnx_shape: [i64; 3usize] = where1_out1;
            let input_dims = constant1_out1.dims();
            let mut shape = onnx_shape;
            #[allow(clippy::needless_range_loop)]
            for i in 0..3usize {
                let dim_offset = i;
                if shape[dim_offset] == 1 && input_dims[i] > 1 {
                    shape[dim_offset] = input_dims[i] as i64;
                }
            }
            constant1_out1.expand(shape)
        };
        let concat3_out1 = burn::tensor::Tensor::cat(
            [expand1_out1, transpose1_out1].into(),
            1,
        );
        let constant417_out1 = 16i64;
        let div3_out1 = gather4_out1 / constant417_out1;
        let constant418_out1 = 16i64;
        let div4_out1 = gather5_out1 / constant418_out1;
        let cast9_out1 = div3_out1 as f32;
        let range1_out1 = {
            let (start, limit, delta) = (0.5f32, cast9_out1, 1f32);
            assert!(delta != 0.0);
            let n = (((limit - start) as f64) / delta as f64).ceil().max(0.0) as i64;
            Tensor::arange(0..n, self.device())
                .cast(burn::tensor::FloatDType::F32)
                .mul_scalar(delta)
                .add_scalar(start)
        };
        let div5_out1 = range1_out1.div_scalar(cast9_out1);
        let cast11_out1 = div4_out1 as f32;
        let range2_out1 = {
            let (start, limit, delta) = (0.5f32, cast11_out1, 1f32);
            assert!(delta != 0.0);
            let n = (((limit - start) as f64) / delta as f64).ceil().max(0.0) as i64;
            Tensor::arange(0..n, self.device())
                .cast(burn::tensor::FloatDType::F32)
                .mul_scalar(delta)
                .add_scalar(start)
        };
        let div6_out1 = range2_out1.div_scalar(cast11_out1);
        let reshape3_out1 = div5_out1.reshape(burn::tensor::ZeroDimPlaceholders([-1]));
        let reshape4_out1 = div6_out1.reshape(burn::tensor::ZeroDimPlaceholders([-1]));
        let shape9_out1: [i64; 1] = {
            let axes = &reshape3_out1.clone().dims()[0..1];
            let mut output = [0i64; 1];
            for i in 0..1 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let shape10_out1: [i64; 1] = {
            let axes = &reshape4_out1.clone().dims()[0..1];
            let mut output = [0i64; 1];
            for i in 0..1 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let concat4_out1: [i64; 2usize] = [&shape9_out1[..], &shape10_out1[..]]
            .concat()
            .try_into()
            .unwrap();
        let constant425_out1: [i64; 1] = [1i64];
        let concat5_out1: [i64; 2usize] = [&shape9_out1[..], &constant425_out1[..]]
            .concat()
            .try_into()
            .unwrap();
        let reshape5_out1 = reshape3_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat5_out1));
        let expand2_out1 = {
            let onnx_shape: [i64; 2usize] = concat4_out1;
            let input_dims = reshape5_out1.dims();
            let mut shape = onnx_shape;
            #[allow(clippy::needless_range_loop)]
            for i in 0..2usize {
                let dim_offset = i;
                if shape[dim_offset] == 1 && input_dims[i] > 1 {
                    shape[dim_offset] = input_dims[i] as i64;
                }
            }
            reshape5_out1.expand(shape)
        };
        let constant426_out1: [i64; 1] = [1i64];
        let concat6_out1: [i64; 2usize] = [&constant426_out1[..], &shape10_out1[..]]
            .concat()
            .try_into()
            .unwrap();
        let reshape6_out1 = reshape4_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat6_out1));
        let expand3_out1 = {
            let onnx_shape: [i64; 2usize] = concat4_out1;
            let input_dims = reshape6_out1.dims();
            let mut shape = onnx_shape;
            #[allow(clippy::needless_range_loop)]
            for i in 0..2usize {
                let dim_offset = i;
                if shape[dim_offset] == 1 && input_dims[i] > 1 {
                    shape[dim_offset] = input_dims[i] as i64;
                }
            }
            reshape6_out1.expand(shape)
        };
        let unsqueeze2_out1: Tensor<3> = expand2_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze3_out1: Tensor<3> = expand3_out1.unsqueeze_dims::<3>(&[-1]);
        let concat7_out1 = burn::tensor::Tensor::cat(
            [unsqueeze2_out1, unsqueeze3_out1].into(),
            2,
        );
        let flatten1_out1 = {
            let leading_dim = concat7_out1.shape()[..2].iter().product::<usize>() as i32;
            concat7_out1.reshape::<2, _>([leading_dim, -1])
        };
        let constant429_out1 = self.constant429.val();
        let mul2_out1 = flatten1_out1.mul((constant429_out1).unsqueeze_dims(&[0isize]));
        let constant430_out1 = self.constant430.val();
        let sub1_out1 = mul2_out1.sub((constant430_out1).unsqueeze_dims(&[0isize]));
        let unsqueeze4_out1: Tensor<3> = sub1_out1.unsqueeze_dims::<3>(&[2]);
        let constant432_out1 = self.constant432.val();
        let mul3_out1 = unsqueeze4_out1
            .mul((constant432_out1).unsqueeze_dims(&[0isize, 1isize]));
        (mul3_out1, concat3_out1, gather3_out1, div1_out1, div2_out1)
    }
}
#[derive(Module, Debug)]
pub struct Submodule2 {
    constant266: burn::module::Param<Tensor<3>>,
    layernormalization1: LayerNorm,
    linear1: Linear,
    constant494: burn::module::Param<Tensor<1>>,
    linear2: Linear,
    device: AllocationDevice,
}
impl Submodule2 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let constant266: burn::module::Param<Tensor<3>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                3,
            >::zeros([1, 1, 16], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1, 1, 16].into(),
        );
        let layernormalization1 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear1 = LinearConfig::new(192, 576).with_bias(true).init(device);
        let constant494: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([1], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1].into(),
        );
        let linear2 = LinearConfig::new(192, 192).with_bias(true).init(device);
        Self {
            constant266,
            layernormalization1,
            linear1,
            constant494,
            linear2,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `mul3_out1`: expected dtype `DType::F32`
    /// - `concat3_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        mul3_out1: Tensor<3>,
        concat3_out1: Tensor<3>,
    ) -> (Tensor<3>, Tensor<4>, Tensor<4>) {
        let constant266_out1 = self.constant266.val();
        let div7_out1 = mul3_out1.div(constant266_out1);
        let flatten2_out1 = {
            let leading_dim = div7_out1.shape()[..1].iter().product::<usize>() as i32;
            div7_out1.reshape::<2, _>([leading_dim, -1])
        };
        let expand4_out1 = {
            let onnx_shape: [i64; 2usize] = [1, 1];
            let input_dims = flatten2_out1.dims();
            let mut shape = onnx_shape;
            #[allow(clippy::needless_range_loop)]
            for i in 0..2usize {
                let dim_offset = i;
                if shape[dim_offset] == 1 && input_dims[i] > 1 {
                    shape[dim_offset] = input_dims[i] as i64;
                }
            }
            flatten2_out1.expand(shape)
        };
        let tile1_out1 = expand4_out1.repeat(&[1, 2]);
        let sin1_out1 = tile1_out1.clone().sin();
        let cos1_out1 = tile1_out1.cos();
        let unsqueeze5_out1: Tensor<3> = sin1_out1.unsqueeze_dims::<3>(&[0]);
        let unsqueeze6_out1: Tensor<4> = unsqueeze5_out1.unsqueeze_dims::<4>(&[0]);
        let unsqueeze7_out1: Tensor<3> = cos1_out1.unsqueeze_dims::<3>(&[0]);
        let unsqueeze8_out1: Tensor<4> = unsqueeze7_out1.unsqueeze_dims::<4>(&[0]);
        let layernormalization1_out1 = {
            let dtype = concat3_out1.clone().dtype();
            self.layernormalization1
                .forward(concat3_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let shape11_out1: [i64; 3] = {
            let axes = &layernormalization1_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather7_out1 = shape11_out1[0] as i64;
        let gather8_out1 = shape11_out1[1] as i64;
        let gather9_out1 = shape11_out1[2] as i64;
        let linear1_out1 = self.linear1.forward(layernormalization1_out1);
        let constant442_out1 = 3i64;
        let div8_out1 = gather9_out1 / constant442_out1;
        let unsqueeze9_out1 = [gather7_out1 as i64];
        let unsqueeze10_out1 = [gather8_out1 as i64];
        let unsqueeze11_out1 = [div8_out1 as i64];
        let constant445_out1: [i64; 1] = [3i64];
        let constant446_out1: [i64; 1] = [3i64];
        let concat8_out1: [i64; 5usize] = [
            &unsqueeze9_out1[..],
            &unsqueeze10_out1[..],
            &constant445_out1[..],
            &constant446_out1[..],
            &unsqueeze11_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape7_out1 = linear1_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat8_out1));
        let transpose2_out1 = reshape7_out1.permute([2, 0, 3, 1, 4]);
        let [split1_out1, split1_out2, split1_out3] = {
            let split_tensors = transpose2_out1.split_with_sizes([1, 1, 1].into(), 0);
            split_tensors.try_into().unwrap()
        };
        let squeeze1_out1 = split1_out1.squeeze_dims::<4>(&[0]);
        let squeeze2_out1 = split1_out2.squeeze_dims::<4>(&[0]);
        let squeeze3_out1 = split1_out3.squeeze_dims::<4>(&[0]);
        let slice2_out1 = squeeze1_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice3_out1 = squeeze1_out1.slice(s![.., .., 1.., ..]);
        let slice4_out1 = squeeze2_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice5_out1 = squeeze2_out1.slice(s![.., .., 1.., ..]);
        let mul4_out1 = slice3_out1.clone().mul(unsqueeze8_out1.clone());
        let shape14_out1: [i64; 4] = {
            let axes = &slice3_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather10_out1 = shape14_out1[3] as i64;
        let constant469_out1 = 2i64;
        let div9_out1 = gather10_out1 / constant469_out1;
        let unsqueeze12_out1 = [div9_out1 as i64];
        let slice6_out1 = slice3_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze12_out1[0]]);
        let gather11_out1 = shape14_out1[3] as i64;
        let constant475_out1 = 2i64;
        let div10_out1 = gather11_out1 / constant475_out1;
        let unsqueeze13_out1 = [div10_out1 as i64];
        let slice7_out1 = slice3_out1
            .slice(s![.., .., .., unsqueeze13_out1[0]..9223372036854775807]);
        let neg1_out1 = slice7_out1.neg();
        let concat9_out1 = burn::tensor::Tensor::cat([neg1_out1, slice6_out1].into(), 3);
        let mul5_out1 = concat9_out1.mul(unsqueeze6_out1.clone());
        let add1_out1 = mul4_out1.add(mul5_out1);
        let mul6_out1 = slice5_out1.clone().mul(unsqueeze8_out1.clone());
        let shape16_out1: [i64; 4] = {
            let axes = &slice5_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather12_out1 = shape16_out1[3] as i64;
        let constant481_out1 = 2i64;
        let div11_out1 = gather12_out1 / constant481_out1;
        let unsqueeze14_out1 = [div11_out1 as i64];
        let slice8_out1 = slice5_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze14_out1[0]]);
        let gather13_out1 = shape16_out1[3] as i64;
        let constant487_out1 = 2i64;
        let div12_out1 = gather13_out1 / constant487_out1;
        let unsqueeze15_out1 = [div12_out1 as i64];
        let slice9_out1 = slice5_out1
            .slice(s![.., .., .., unsqueeze15_out1[0]..9223372036854775807]);
        let neg2_out1 = slice9_out1.neg();
        let concat10_out1 = burn::tensor::Tensor::cat(
            [neg2_out1, slice8_out1].into(),
            3,
        );
        let mul7_out1 = concat10_out1.mul(unsqueeze6_out1.clone());
        let add2_out1 = mul6_out1.add(mul7_out1);
        let concat11_out1 = burn::tensor::Tensor::cat(
            [slice2_out1, add1_out1].into(),
            2,
        );
        let concat12_out1 = burn::tensor::Tensor::cat(
            [slice4_out1, add2_out1].into(),
            2,
        );
        let shape18_out1: [i64; 4] = {
            let axes = &concat11_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice10_out1: [i64; 1] = shape18_out1[3..4].try_into().unwrap();
        let cast23_out1 = {
            let shape_array = slice10_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<
                1,
            >::from_data(
                TensorData::from(float_array),
                (self.device(), burn::tensor::DType::F32),
            )
        };
        let sqrt1_out1 = cast23_out1.sqrt();
        let constant494_out1 = self.constant494.val();
        let div13_out1 = constant494_out1.div(sqrt1_out1);
        let transpose3_out1 = concat12_out1.permute([0, 1, 3, 2]);
        let sqrt2_out1 = div13_out1.sqrt();
        let mul8_out1 = concat11_out1
            .mul((sqrt2_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul9_out1 = transpose3_out1
            .mul((sqrt2_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul2_out1 = mul8_out1.matmul(mul9_out1);
        let softmax1_out1 = burn::tensor::activation::softmax(matmul2_out1, 3);
        let matmul3_out1 = softmax1_out1.matmul(squeeze3_out1);
        let transpose4_out1 = matmul3_out1.permute([0, 2, 1, 3]);
        let unsqueeze16_out1 = [gather7_out1 as i64];
        let unsqueeze17_out1 = [gather8_out1 as i64];
        let unsqueeze18_out1 = [gather9_out1 as i64];
        let concat13_out1: [i64; 3usize] = [
            &unsqueeze16_out1[..],
            &unsqueeze17_out1[..],
            &unsqueeze18_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape8_out1 = transpose4_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat13_out1));
        let linear2_out1 = self.linear2.forward(reshape8_out1);
        let add3_out1 = concat3_out1.add(linear2_out1);
        (add3_out1, unsqueeze8_out1, unsqueeze6_out1)
    }
}
#[derive(Module, Debug)]
pub struct Submodule3 {
    layernormalization2: LayerNorm,
    linear3: Linear,
    constant498: burn::module::Param<Tensor<1>>,
    constant499: burn::module::Param<Tensor<1>>,
    constant500: burn::module::Param<Tensor<1>>,
    linear4: Linear,
    layernormalization3: LayerNorm,
    linear5: Linear,
    constant556: burn::module::Param<Tensor<1>>,
    linear6: Linear,
    device: AllocationDevice,
}
impl Submodule3 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization2 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear3 = LinearConfig::new(192, 768).with_bias(true).init(device);
        let constant498: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1.4142135381698608f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant499: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant500: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear4 = LinearConfig::new(768, 192).with_bias(true).init(device);
        let layernormalization3 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear5 = LinearConfig::new(192, 576).with_bias(true).init(device);
        let constant556: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([1], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1].into(),
        );
        let linear6 = LinearConfig::new(192, 192).with_bias(true).init(device);
        Self {
            layernormalization2,
            linear3,
            constant498,
            constant499,
            constant500,
            linear4,
            layernormalization3,
            linear5,
            constant556,
            linear6,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `add3_out1`: expected dtype `DType::F32`
    /// - `unsqueeze8_out1`: expected dtype `DType::F32`
    /// - `unsqueeze6_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        add3_out1: Tensor<3>,
        unsqueeze8_out1: Tensor<4>,
        unsqueeze6_out1: Tensor<4>,
    ) -> Tensor<3> {
        let layernormalization2_out1 = {
            let dtype = add3_out1.clone().dtype();
            self.layernormalization2
                .forward(add3_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear3_out1 = self.linear3.forward(layernormalization2_out1);
        let constant498_out1 = self.constant498.val();
        let div14_out1 = linear3_out1
            .clone()
            .div((constant498_out1).unsqueeze_dims(&[0isize, 1isize]));
        let erf1_out1 = div14_out1.erf();
        let constant499_out1 = self.constant499.val();
        let add4_out1 = erf1_out1
            .add((constant499_out1).unsqueeze_dims(&[0isize, 1isize]));
        let mul10_out1 = linear3_out1.mul(add4_out1);
        let constant500_out1 = self.constant500.val();
        let mul11_out1 = mul10_out1
            .mul((constant500_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear4_out1 = self.linear4.forward(mul11_out1);
        let add5_out1 = add3_out1.add(linear4_out1);
        let layernormalization3_out1 = {
            let dtype = add5_out1.clone().dtype();
            self.layernormalization3
                .forward(add5_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let shape19_out1: [i64; 3] = {
            let axes = &layernormalization3_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather14_out1 = shape19_out1[0] as i64;
        let gather15_out1 = shape19_out1[1] as i64;
        let gather16_out1 = shape19_out1[2] as i64;
        let linear5_out1 = self.linear5.forward(layernormalization3_out1);
        let constant504_out1 = 3i64;
        let div15_out1 = gather16_out1 / constant504_out1;
        let unsqueeze19_out1 = [gather14_out1 as i64];
        let unsqueeze20_out1 = [gather15_out1 as i64];
        let unsqueeze21_out1 = [div15_out1 as i64];
        let constant507_out1: [i64; 1] = [3i64];
        let constant508_out1: [i64; 1] = [3i64];
        let concat14_out1: [i64; 5usize] = [
            &unsqueeze19_out1[..],
            &unsqueeze20_out1[..],
            &constant507_out1[..],
            &constant508_out1[..],
            &unsqueeze21_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape9_out1 = linear5_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat14_out1));
        let transpose5_out1 = reshape9_out1.permute([2, 0, 3, 1, 4]);
        let [split2_out1, split2_out2, split2_out3] = {
            let split_tensors = transpose5_out1.split_with_sizes([1, 1, 1].into(), 0);
            split_tensors.try_into().unwrap()
        };
        let squeeze4_out1 = split2_out1.squeeze_dims::<4>(&[0]);
        let squeeze5_out1 = split2_out2.squeeze_dims::<4>(&[0]);
        let squeeze6_out1 = split2_out3.squeeze_dims::<4>(&[0]);
        let slice11_out1 = squeeze4_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice12_out1 = squeeze4_out1.slice(s![.., .., 1.., ..]);
        let slice13_out1 = squeeze5_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice14_out1 = squeeze5_out1.slice(s![.., .., 1.., ..]);
        let mul12_out1 = slice12_out1.clone().mul(unsqueeze8_out1.clone());
        let shape22_out1: [i64; 4] = {
            let axes = &slice12_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather17_out1 = shape22_out1[3] as i64;
        let constant531_out1 = 2i64;
        let div16_out1 = gather17_out1 / constant531_out1;
        let unsqueeze22_out1 = [div16_out1 as i64];
        let slice15_out1 = slice12_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze22_out1[0]]);
        let gather18_out1 = shape22_out1[3] as i64;
        let constant537_out1 = 2i64;
        let div17_out1 = gather18_out1 / constant537_out1;
        let unsqueeze23_out1 = [div17_out1 as i64];
        let slice16_out1 = slice12_out1
            .slice(s![.., .., .., unsqueeze23_out1[0]..9223372036854775807]);
        let neg3_out1 = slice16_out1.neg();
        let concat15_out1 = burn::tensor::Tensor::cat(
            [neg3_out1, slice15_out1].into(),
            3,
        );
        let mul13_out1 = concat15_out1.mul(unsqueeze6_out1.clone());
        let add6_out1 = mul12_out1.add(mul13_out1);
        let mul14_out1 = slice14_out1.clone().mul(unsqueeze8_out1);
        let shape24_out1: [i64; 4] = {
            let axes = &slice14_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather19_out1 = shape24_out1[3] as i64;
        let constant543_out1 = 2i64;
        let div18_out1 = gather19_out1 / constant543_out1;
        let unsqueeze24_out1 = [div18_out1 as i64];
        let slice17_out1 = slice14_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze24_out1[0]]);
        let gather20_out1 = shape24_out1[3] as i64;
        let constant549_out1 = 2i64;
        let div19_out1 = gather20_out1 / constant549_out1;
        let unsqueeze25_out1 = [div19_out1 as i64];
        let slice18_out1 = slice14_out1
            .slice(s![.., .., .., unsqueeze25_out1[0]..9223372036854775807]);
        let neg4_out1 = slice18_out1.neg();
        let concat16_out1 = burn::tensor::Tensor::cat(
            [neg4_out1, slice17_out1].into(),
            3,
        );
        let mul15_out1 = concat16_out1.mul(unsqueeze6_out1);
        let add7_out1 = mul14_out1.add(mul15_out1);
        let concat17_out1 = burn::tensor::Tensor::cat(
            [slice11_out1, add6_out1].into(),
            2,
        );
        let concat18_out1 = burn::tensor::Tensor::cat(
            [slice13_out1, add7_out1].into(),
            2,
        );
        let shape26_out1: [i64; 4] = {
            let axes = &concat17_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice19_out1: [i64; 1] = shape26_out1[3..4].try_into().unwrap();
        let cast35_out1 = {
            let shape_array = slice19_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<
                1,
            >::from_data(
                TensorData::from(float_array),
                (self.device(), burn::tensor::DType::F32),
            )
        };
        let sqrt4_out1 = cast35_out1.sqrt();
        let constant556_out1 = self.constant556.val();
        let div20_out1 = constant556_out1.div(sqrt4_out1);
        let transpose6_out1 = concat18_out1.permute([0, 1, 3, 2]);
        let sqrt5_out1 = div20_out1.sqrt();
        let mul16_out1 = concat17_out1
            .mul((sqrt5_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul17_out1 = transpose6_out1
            .mul((sqrt5_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul8_out1 = mul16_out1.matmul(mul17_out1);
        let softmax2_out1 = burn::tensor::activation::softmax(matmul8_out1, 3);
        let matmul9_out1 = softmax2_out1.matmul(squeeze6_out1);
        let transpose7_out1 = matmul9_out1.permute([0, 2, 1, 3]);
        let unsqueeze26_out1 = [gather14_out1 as i64];
        let unsqueeze27_out1 = [gather15_out1 as i64];
        let unsqueeze28_out1 = [gather16_out1 as i64];
        let concat19_out1: [i64; 3usize] = [
            &unsqueeze26_out1[..],
            &unsqueeze27_out1[..],
            &unsqueeze28_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape10_out1 = transpose7_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat19_out1));
        let linear6_out1 = self.linear6.forward(reshape10_out1);
        let add8_out1 = add5_out1.add(linear6_out1);
        add8_out1
    }
}
#[derive(Module, Debug)]
pub struct Submodule4 {
    layernormalization4: LayerNorm,
    linear7: Linear,
    constant560: burn::module::Param<Tensor<1>>,
    constant561: burn::module::Param<Tensor<1>>,
    constant562: burn::module::Param<Tensor<1>>,
    linear8: Linear,
    layernormalization5: LayerNorm,
    linear9: Linear,
    constant618: burn::module::Param<Tensor<1>>,
    linear10: Linear,
    device: AllocationDevice,
}
impl Submodule4 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization4 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear7 = LinearConfig::new(192, 768).with_bias(true).init(device);
        let constant560: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1.4142135381698608f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant561: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant562: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear8 = LinearConfig::new(768, 192).with_bias(true).init(device);
        let layernormalization5 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear9 = LinearConfig::new(192, 576).with_bias(true).init(device);
        let constant618: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([1], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1].into(),
        );
        let linear10 = LinearConfig::new(192, 192).with_bias(true).init(device);
        Self {
            layernormalization4,
            linear7,
            constant560,
            constant561,
            constant562,
            linear8,
            layernormalization5,
            linear9,
            constant618,
            linear10,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `add8_out1`: expected dtype `DType::F32`
    /// - `unsqueeze8_out1`: expected dtype `DType::F32`
    /// - `unsqueeze6_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        add8_out1: Tensor<3>,
        unsqueeze8_out1: Tensor<4>,
        unsqueeze6_out1: Tensor<4>,
    ) -> Tensor<3> {
        let layernormalization4_out1 = {
            let dtype = add8_out1.clone().dtype();
            self.layernormalization4
                .forward(add8_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear7_out1 = self.linear7.forward(layernormalization4_out1);
        let constant560_out1 = self.constant560.val();
        let div21_out1 = linear7_out1
            .clone()
            .div((constant560_out1).unsqueeze_dims(&[0isize, 1isize]));
        let erf2_out1 = div21_out1.erf();
        let constant561_out1 = self.constant561.val();
        let add9_out1 = erf2_out1
            .add((constant561_out1).unsqueeze_dims(&[0isize, 1isize]));
        let mul18_out1 = linear7_out1.mul(add9_out1);
        let constant562_out1 = self.constant562.val();
        let mul19_out1 = mul18_out1
            .mul((constant562_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear8_out1 = self.linear8.forward(mul19_out1);
        let add10_out1 = add8_out1.add(linear8_out1);
        let layernormalization5_out1 = {
            let dtype = add10_out1.clone().dtype();
            self.layernormalization5
                .forward(add10_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let shape27_out1: [i64; 3] = {
            let axes = &layernormalization5_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather21_out1 = shape27_out1[0] as i64;
        let gather22_out1 = shape27_out1[1] as i64;
        let gather23_out1 = shape27_out1[2] as i64;
        let linear9_out1 = self.linear9.forward(layernormalization5_out1);
        let constant566_out1 = 3i64;
        let div22_out1 = gather23_out1 / constant566_out1;
        let unsqueeze29_out1 = [gather21_out1 as i64];
        let unsqueeze30_out1 = [gather22_out1 as i64];
        let unsqueeze31_out1 = [div22_out1 as i64];
        let constant569_out1: [i64; 1] = [3i64];
        let constant570_out1: [i64; 1] = [3i64];
        let concat20_out1: [i64; 5usize] = [
            &unsqueeze29_out1[..],
            &unsqueeze30_out1[..],
            &constant569_out1[..],
            &constant570_out1[..],
            &unsqueeze31_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape11_out1 = linear9_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat20_out1));
        let transpose8_out1 = reshape11_out1.permute([2, 0, 3, 1, 4]);
        let [split3_out1, split3_out2, split3_out3] = {
            let split_tensors = transpose8_out1.split_with_sizes([1, 1, 1].into(), 0);
            split_tensors.try_into().unwrap()
        };
        let squeeze7_out1 = split3_out1.squeeze_dims::<4>(&[0]);
        let squeeze8_out1 = split3_out2.squeeze_dims::<4>(&[0]);
        let squeeze9_out1 = split3_out3.squeeze_dims::<4>(&[0]);
        let slice20_out1 = squeeze7_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice21_out1 = squeeze7_out1.slice(s![.., .., 1.., ..]);
        let slice22_out1 = squeeze8_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice23_out1 = squeeze8_out1.slice(s![.., .., 1.., ..]);
        let mul20_out1 = slice21_out1.clone().mul(unsqueeze8_out1.clone());
        let shape30_out1: [i64; 4] = {
            let axes = &slice21_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather24_out1 = shape30_out1[3] as i64;
        let constant593_out1 = 2i64;
        let div23_out1 = gather24_out1 / constant593_out1;
        let unsqueeze32_out1 = [div23_out1 as i64];
        let slice24_out1 = slice21_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze32_out1[0]]);
        let gather25_out1 = shape30_out1[3] as i64;
        let constant599_out1 = 2i64;
        let div24_out1 = gather25_out1 / constant599_out1;
        let unsqueeze33_out1 = [div24_out1 as i64];
        let slice25_out1 = slice21_out1
            .slice(s![.., .., .., unsqueeze33_out1[0]..9223372036854775807]);
        let neg5_out1 = slice25_out1.neg();
        let concat21_out1 = burn::tensor::Tensor::cat(
            [neg5_out1, slice24_out1].into(),
            3,
        );
        let mul21_out1 = concat21_out1.mul(unsqueeze6_out1.clone());
        let add11_out1 = mul20_out1.add(mul21_out1);
        let mul22_out1 = slice23_out1.clone().mul(unsqueeze8_out1);
        let shape32_out1: [i64; 4] = {
            let axes = &slice23_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather26_out1 = shape32_out1[3] as i64;
        let constant605_out1 = 2i64;
        let div25_out1 = gather26_out1 / constant605_out1;
        let unsqueeze34_out1 = [div25_out1 as i64];
        let slice26_out1 = slice23_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze34_out1[0]]);
        let gather27_out1 = shape32_out1[3] as i64;
        let constant611_out1 = 2i64;
        let div26_out1 = gather27_out1 / constant611_out1;
        let unsqueeze35_out1 = [div26_out1 as i64];
        let slice27_out1 = slice23_out1
            .slice(s![.., .., .., unsqueeze35_out1[0]..9223372036854775807]);
        let neg6_out1 = slice27_out1.neg();
        let concat22_out1 = burn::tensor::Tensor::cat(
            [neg6_out1, slice26_out1].into(),
            3,
        );
        let mul23_out1 = concat22_out1.mul(unsqueeze6_out1);
        let add12_out1 = mul22_out1.add(mul23_out1);
        let concat23_out1 = burn::tensor::Tensor::cat(
            [slice20_out1, add11_out1].into(),
            2,
        );
        let concat24_out1 = burn::tensor::Tensor::cat(
            [slice22_out1, add12_out1].into(),
            2,
        );
        let shape34_out1: [i64; 4] = {
            let axes = &concat23_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice28_out1: [i64; 1] = shape34_out1[3..4].try_into().unwrap();
        let cast47_out1 = {
            let shape_array = slice28_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<
                1,
            >::from_data(
                TensorData::from(float_array),
                (self.device(), burn::tensor::DType::F32),
            )
        };
        let sqrt7_out1 = cast47_out1.sqrt();
        let constant618_out1 = self.constant618.val();
        let div27_out1 = constant618_out1.div(sqrt7_out1);
        let transpose9_out1 = concat24_out1.permute([0, 1, 3, 2]);
        let sqrt8_out1 = div27_out1.sqrt();
        let mul24_out1 = concat23_out1
            .mul((sqrt8_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul25_out1 = transpose9_out1
            .mul((sqrt8_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul14_out1 = mul24_out1.matmul(mul25_out1);
        let softmax3_out1 = burn::tensor::activation::softmax(matmul14_out1, 3);
        let matmul15_out1 = softmax3_out1.matmul(squeeze9_out1);
        let transpose10_out1 = matmul15_out1.permute([0, 2, 1, 3]);
        let unsqueeze36_out1 = [gather21_out1 as i64];
        let unsqueeze37_out1 = [gather22_out1 as i64];
        let unsqueeze38_out1 = [gather23_out1 as i64];
        let concat25_out1: [i64; 3usize] = [
            &unsqueeze36_out1[..],
            &unsqueeze37_out1[..],
            &unsqueeze38_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape12_out1 = transpose10_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat25_out1));
        let linear10_out1 = self.linear10.forward(reshape12_out1);
        let add13_out1 = add10_out1.add(linear10_out1);
        add13_out1
    }
}
#[derive(Module, Debug)]
pub struct Submodule5 {
    layernormalization6: LayerNorm,
    linear11: Linear,
    constant622: burn::module::Param<Tensor<1>>,
    constant623: burn::module::Param<Tensor<1>>,
    constant624: burn::module::Param<Tensor<1>>,
    linear12: Linear,
    layernormalization7: LayerNorm,
    linear13: Linear,
    constant680: burn::module::Param<Tensor<1>>,
    linear14: Linear,
    device: AllocationDevice,
}
impl Submodule5 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization6 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear11 = LinearConfig::new(192, 768).with_bias(true).init(device);
        let constant622: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1.4142135381698608f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant623: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant624: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear12 = LinearConfig::new(768, 192).with_bias(true).init(device);
        let layernormalization7 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear13 = LinearConfig::new(192, 576).with_bias(true).init(device);
        let constant680: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([1], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1].into(),
        );
        let linear14 = LinearConfig::new(192, 192).with_bias(true).init(device);
        Self {
            layernormalization6,
            linear11,
            constant622,
            constant623,
            constant624,
            linear12,
            layernormalization7,
            linear13,
            constant680,
            linear14,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `add13_out1`: expected dtype `DType::F32`
    /// - `unsqueeze8_out1`: expected dtype `DType::F32`
    /// - `unsqueeze6_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        add13_out1: Tensor<3>,
        unsqueeze8_out1: Tensor<4>,
        unsqueeze6_out1: Tensor<4>,
    ) -> Tensor<3> {
        let layernormalization6_out1 = {
            let dtype = add13_out1.clone().dtype();
            self.layernormalization6
                .forward(add13_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear11_out1 = self.linear11.forward(layernormalization6_out1);
        let constant622_out1 = self.constant622.val();
        let div28_out1 = linear11_out1
            .clone()
            .div((constant622_out1).unsqueeze_dims(&[0isize, 1isize]));
        let erf3_out1 = div28_out1.erf();
        let constant623_out1 = self.constant623.val();
        let add14_out1 = erf3_out1
            .add((constant623_out1).unsqueeze_dims(&[0isize, 1isize]));
        let mul26_out1 = linear11_out1.mul(add14_out1);
        let constant624_out1 = self.constant624.val();
        let mul27_out1 = mul26_out1
            .mul((constant624_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear12_out1 = self.linear12.forward(mul27_out1);
        let add15_out1 = add13_out1.add(linear12_out1);
        let layernormalization7_out1 = {
            let dtype = add15_out1.clone().dtype();
            self.layernormalization7
                .forward(add15_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let shape35_out1: [i64; 3] = {
            let axes = &layernormalization7_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather28_out1 = shape35_out1[0] as i64;
        let gather29_out1 = shape35_out1[1] as i64;
        let gather30_out1 = shape35_out1[2] as i64;
        let linear13_out1 = self.linear13.forward(layernormalization7_out1);
        let constant628_out1 = 3i64;
        let div29_out1 = gather30_out1 / constant628_out1;
        let unsqueeze39_out1 = [gather28_out1 as i64];
        let unsqueeze40_out1 = [gather29_out1 as i64];
        let unsqueeze41_out1 = [div29_out1 as i64];
        let constant631_out1: [i64; 1] = [3i64];
        let constant632_out1: [i64; 1] = [3i64];
        let concat26_out1: [i64; 5usize] = [
            &unsqueeze39_out1[..],
            &unsqueeze40_out1[..],
            &constant631_out1[..],
            &constant632_out1[..],
            &unsqueeze41_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape13_out1 = linear13_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat26_out1));
        let transpose11_out1 = reshape13_out1.permute([2, 0, 3, 1, 4]);
        let [split4_out1, split4_out2, split4_out3] = {
            let split_tensors = transpose11_out1.split_with_sizes([1, 1, 1].into(), 0);
            split_tensors.try_into().unwrap()
        };
        let squeeze10_out1 = split4_out1.squeeze_dims::<4>(&[0]);
        let squeeze11_out1 = split4_out2.squeeze_dims::<4>(&[0]);
        let squeeze12_out1 = split4_out3.squeeze_dims::<4>(&[0]);
        let slice29_out1 = squeeze10_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice30_out1 = squeeze10_out1.slice(s![.., .., 1.., ..]);
        let slice31_out1 = squeeze11_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice32_out1 = squeeze11_out1.slice(s![.., .., 1.., ..]);
        let mul28_out1 = slice30_out1.clone().mul(unsqueeze8_out1.clone());
        let shape38_out1: [i64; 4] = {
            let axes = &slice30_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather31_out1 = shape38_out1[3] as i64;
        let constant655_out1 = 2i64;
        let div30_out1 = gather31_out1 / constant655_out1;
        let unsqueeze42_out1 = [div30_out1 as i64];
        let slice33_out1 = slice30_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze42_out1[0]]);
        let gather32_out1 = shape38_out1[3] as i64;
        let constant661_out1 = 2i64;
        let div31_out1 = gather32_out1 / constant661_out1;
        let unsqueeze43_out1 = [div31_out1 as i64];
        let slice34_out1 = slice30_out1
            .slice(s![.., .., .., unsqueeze43_out1[0]..9223372036854775807]);
        let neg7_out1 = slice34_out1.neg();
        let concat27_out1 = burn::tensor::Tensor::cat(
            [neg7_out1, slice33_out1].into(),
            3,
        );
        let mul29_out1 = concat27_out1.mul(unsqueeze6_out1.clone());
        let add16_out1 = mul28_out1.add(mul29_out1);
        let mul30_out1 = slice32_out1.clone().mul(unsqueeze8_out1);
        let shape40_out1: [i64; 4] = {
            let axes = &slice32_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather33_out1 = shape40_out1[3] as i64;
        let constant667_out1 = 2i64;
        let div32_out1 = gather33_out1 / constant667_out1;
        let unsqueeze44_out1 = [div32_out1 as i64];
        let slice35_out1 = slice32_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze44_out1[0]]);
        let gather34_out1 = shape40_out1[3] as i64;
        let constant673_out1 = 2i64;
        let div33_out1 = gather34_out1 / constant673_out1;
        let unsqueeze45_out1 = [div33_out1 as i64];
        let slice36_out1 = slice32_out1
            .slice(s![.., .., .., unsqueeze45_out1[0]..9223372036854775807]);
        let neg8_out1 = slice36_out1.neg();
        let concat28_out1 = burn::tensor::Tensor::cat(
            [neg8_out1, slice35_out1].into(),
            3,
        );
        let mul31_out1 = concat28_out1.mul(unsqueeze6_out1);
        let add17_out1 = mul30_out1.add(mul31_out1);
        let concat29_out1 = burn::tensor::Tensor::cat(
            [slice29_out1, add16_out1].into(),
            2,
        );
        let concat30_out1 = burn::tensor::Tensor::cat(
            [slice31_out1, add17_out1].into(),
            2,
        );
        let shape42_out1: [i64; 4] = {
            let axes = &concat29_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice37_out1: [i64; 1] = shape42_out1[3..4].try_into().unwrap();
        let cast59_out1 = {
            let shape_array = slice37_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<
                1,
            >::from_data(
                TensorData::from(float_array),
                (self.device(), burn::tensor::DType::F32),
            )
        };
        let sqrt10_out1 = cast59_out1.sqrt();
        let constant680_out1 = self.constant680.val();
        let div34_out1 = constant680_out1.div(sqrt10_out1);
        let transpose12_out1 = concat30_out1.permute([0, 1, 3, 2]);
        let sqrt11_out1 = div34_out1.sqrt();
        let mul32_out1 = concat29_out1
            .mul((sqrt11_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul33_out1 = transpose12_out1
            .mul((sqrt11_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul20_out1 = mul32_out1.matmul(mul33_out1);
        let softmax4_out1 = burn::tensor::activation::softmax(matmul20_out1, 3);
        let matmul21_out1 = softmax4_out1.matmul(squeeze12_out1);
        let transpose13_out1 = matmul21_out1.permute([0, 2, 1, 3]);
        let unsqueeze46_out1 = [gather28_out1 as i64];
        let unsqueeze47_out1 = [gather29_out1 as i64];
        let unsqueeze48_out1 = [gather30_out1 as i64];
        let concat31_out1: [i64; 3usize] = [
            &unsqueeze46_out1[..],
            &unsqueeze47_out1[..],
            &unsqueeze48_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape14_out1 = transpose13_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat31_out1));
        let linear14_out1 = self.linear14.forward(reshape14_out1);
        let add18_out1 = add15_out1.add(linear14_out1);
        add18_out1
    }
}
#[derive(Module, Debug)]
pub struct Submodule6 {
    layernormalization8: LayerNorm,
    linear15: Linear,
    constant684: burn::module::Param<Tensor<1>>,
    constant685: burn::module::Param<Tensor<1>>,
    constant686: burn::module::Param<Tensor<1>>,
    linear16: Linear,
    layernormalization9: LayerNorm,
    linear17: Linear,
    constant746: burn::module::Param<Tensor<1>>,
    linear18: Linear,
    device: AllocationDevice,
}
impl Submodule6 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization8 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear15 = LinearConfig::new(192, 768).with_bias(true).init(device);
        let constant684: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1.4142135381698608f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant685: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant686: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear16 = LinearConfig::new(768, 192).with_bias(true).init(device);
        let layernormalization9 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear17 = LinearConfig::new(192, 576).with_bias(true).init(device);
        let constant746: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([1], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1].into(),
        );
        let linear18 = LinearConfig::new(192, 192).with_bias(true).init(device);
        Self {
            layernormalization8,
            linear15,
            constant684,
            constant685,
            constant686,
            linear16,
            layernormalization9,
            linear17,
            constant746,
            linear18,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `add18_out1`: expected dtype `DType::F32`
    /// - `unsqueeze8_out1`: expected dtype `DType::F32`
    /// - `unsqueeze6_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        add18_out1: Tensor<3>,
        unsqueeze8_out1: Tensor<4>,
        unsqueeze6_out1: Tensor<4>,
    ) -> (Tensor<3>, Tensor<3>) {
        let layernormalization8_out1 = {
            let dtype = add18_out1.clone().dtype();
            self.layernormalization8
                .forward(add18_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear15_out1 = self.linear15.forward(layernormalization8_out1);
        let constant684_out1 = self.constant684.val();
        let div35_out1 = linear15_out1
            .clone()
            .div((constant684_out1).unsqueeze_dims(&[0isize, 1isize]));
        let erf4_out1 = div35_out1.erf();
        let constant685_out1 = self.constant685.val();
        let add19_out1 = erf4_out1
            .add((constant685_out1).unsqueeze_dims(&[0isize, 1isize]));
        let mul34_out1 = linear15_out1.mul(add19_out1);
        let constant686_out1 = self.constant686.val();
        let mul35_out1 = mul34_out1
            .mul((constant686_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear16_out1 = self.linear16.forward(mul35_out1);
        let add20_out1 = add18_out1.add(linear16_out1);
        let slice38_out1 = add20_out1.clone().slice(s![.., 1.., ..]);
        let layernormalization9_out1 = {
            let dtype = add20_out1.clone().dtype();
            self.layernormalization9
                .forward(add20_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let shape43_out1: [i64; 3] = {
            let axes = &layernormalization9_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather35_out1 = shape43_out1[0] as i64;
        let gather36_out1 = shape43_out1[1] as i64;
        let gather37_out1 = shape43_out1[2] as i64;
        let linear17_out1 = self.linear17.forward(layernormalization9_out1);
        let constant694_out1 = 3i64;
        let div36_out1 = gather37_out1 / constant694_out1;
        let unsqueeze49_out1 = [gather35_out1 as i64];
        let unsqueeze50_out1 = [gather36_out1 as i64];
        let unsqueeze51_out1 = [div36_out1 as i64];
        let constant697_out1: [i64; 1] = [3i64];
        let constant698_out1: [i64; 1] = [3i64];
        let concat32_out1: [i64; 5usize] = [
            &unsqueeze49_out1[..],
            &unsqueeze50_out1[..],
            &constant697_out1[..],
            &constant698_out1[..],
            &unsqueeze51_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape15_out1 = linear17_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat32_out1));
        let transpose14_out1 = reshape15_out1.permute([2, 0, 3, 1, 4]);
        let [split5_out1, split5_out2, split5_out3] = {
            let split_tensors = transpose14_out1.split_with_sizes([1, 1, 1].into(), 0);
            split_tensors.try_into().unwrap()
        };
        let squeeze13_out1 = split5_out1.squeeze_dims::<4>(&[0]);
        let squeeze14_out1 = split5_out2.squeeze_dims::<4>(&[0]);
        let squeeze15_out1 = split5_out3.squeeze_dims::<4>(&[0]);
        let slice39_out1 = squeeze13_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice40_out1 = squeeze13_out1.slice(s![.., .., 1.., ..]);
        let slice41_out1 = squeeze14_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice42_out1 = squeeze14_out1.slice(s![.., .., 1.., ..]);
        let mul36_out1 = slice40_out1.clone().mul(unsqueeze8_out1.clone());
        let shape46_out1: [i64; 4] = {
            let axes = &slice40_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather38_out1 = shape46_out1[3] as i64;
        let constant721_out1 = 2i64;
        let div37_out1 = gather38_out1 / constant721_out1;
        let unsqueeze52_out1 = [div37_out1 as i64];
        let slice43_out1 = slice40_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze52_out1[0]]);
        let gather39_out1 = shape46_out1[3] as i64;
        let constant727_out1 = 2i64;
        let div38_out1 = gather39_out1 / constant727_out1;
        let unsqueeze53_out1 = [div38_out1 as i64];
        let slice44_out1 = slice40_out1
            .slice(s![.., .., .., unsqueeze53_out1[0]..9223372036854775807]);
        let neg9_out1 = slice44_out1.neg();
        let concat33_out1 = burn::tensor::Tensor::cat(
            [neg9_out1, slice43_out1].into(),
            3,
        );
        let mul37_out1 = concat33_out1.mul(unsqueeze6_out1.clone());
        let add21_out1 = mul36_out1.add(mul37_out1);
        let mul38_out1 = slice42_out1.clone().mul(unsqueeze8_out1);
        let shape48_out1: [i64; 4] = {
            let axes = &slice42_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather40_out1 = shape48_out1[3] as i64;
        let constant733_out1 = 2i64;
        let div39_out1 = gather40_out1 / constant733_out1;
        let unsqueeze54_out1 = [div39_out1 as i64];
        let slice45_out1 = slice42_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze54_out1[0]]);
        let gather41_out1 = shape48_out1[3] as i64;
        let constant739_out1 = 2i64;
        let div40_out1 = gather41_out1 / constant739_out1;
        let unsqueeze55_out1 = [div40_out1 as i64];
        let slice46_out1 = slice42_out1
            .slice(s![.., .., .., unsqueeze55_out1[0]..9223372036854775807]);
        let neg10_out1 = slice46_out1.neg();
        let concat34_out1 = burn::tensor::Tensor::cat(
            [neg10_out1, slice45_out1].into(),
            3,
        );
        let mul39_out1 = concat34_out1.mul(unsqueeze6_out1);
        let add22_out1 = mul38_out1.add(mul39_out1);
        let concat35_out1 = burn::tensor::Tensor::cat(
            [slice39_out1, add21_out1].into(),
            2,
        );
        let concat36_out1 = burn::tensor::Tensor::cat(
            [slice41_out1, add22_out1].into(),
            2,
        );
        let shape50_out1: [i64; 4] = {
            let axes = &concat35_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice47_out1: [i64; 1] = shape50_out1[3..4].try_into().unwrap();
        let cast71_out1 = {
            let shape_array = slice47_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<
                1,
            >::from_data(
                TensorData::from(float_array),
                (self.device(), burn::tensor::DType::F32),
            )
        };
        let sqrt13_out1 = cast71_out1.sqrt();
        let constant746_out1 = self.constant746.val();
        let div41_out1 = constant746_out1.div(sqrt13_out1);
        let transpose15_out1 = concat36_out1.permute([0, 1, 3, 2]);
        let sqrt14_out1 = div41_out1.sqrt();
        let mul40_out1 = concat35_out1
            .mul((sqrt14_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul41_out1 = transpose15_out1
            .mul((sqrt14_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul26_out1 = mul40_out1.matmul(mul41_out1);
        let softmax5_out1 = burn::tensor::activation::softmax(matmul26_out1, 3);
        let matmul27_out1 = softmax5_out1.matmul(squeeze15_out1);
        let transpose16_out1 = matmul27_out1.permute([0, 2, 1, 3]);
        let unsqueeze56_out1 = [gather35_out1 as i64];
        let unsqueeze57_out1 = [gather36_out1 as i64];
        let unsqueeze58_out1 = [gather37_out1 as i64];
        let concat37_out1: [i64; 3usize] = [
            &unsqueeze56_out1[..],
            &unsqueeze57_out1[..],
            &unsqueeze58_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape16_out1 = transpose16_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat37_out1));
        let linear18_out1 = self.linear18.forward(reshape16_out1);
        let add23_out1 = add20_out1.add(linear18_out1);
        (add23_out1, slice38_out1)
    }
}
#[derive(Module, Debug)]
pub struct Submodule7 {
    layernormalization10: LayerNorm,
    linear19: Linear,
    constant750: burn::module::Param<Tensor<1>>,
    constant751: burn::module::Param<Tensor<1>>,
    constant752: burn::module::Param<Tensor<1>>,
    linear20: Linear,
    layernormalization11: LayerNorm,
    linear21: Linear,
    constant808: burn::module::Param<Tensor<1>>,
    linear22: Linear,
    device: AllocationDevice,
}
impl Submodule7 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization10 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear19 = LinearConfig::new(192, 768).with_bias(true).init(device);
        let constant750: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1.4142135381698608f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant751: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant752: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear20 = LinearConfig::new(768, 192).with_bias(true).init(device);
        let layernormalization11 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear21 = LinearConfig::new(192, 576).with_bias(true).init(device);
        let constant808: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([1], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1].into(),
        );
        let linear22 = LinearConfig::new(192, 192).with_bias(true).init(device);
        Self {
            layernormalization10,
            linear19,
            constant750,
            constant751,
            constant752,
            linear20,
            layernormalization11,
            linear21,
            constant808,
            linear22,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `add23_out1`: expected dtype `DType::F32`
    /// - `unsqueeze8_out1`: expected dtype `DType::F32`
    /// - `unsqueeze6_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        add23_out1: Tensor<3>,
        unsqueeze8_out1: Tensor<4>,
        unsqueeze6_out1: Tensor<4>,
    ) -> Tensor<3> {
        let layernormalization10_out1 = {
            let dtype = add23_out1.clone().dtype();
            self.layernormalization10
                .forward(add23_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear19_out1 = self.linear19.forward(layernormalization10_out1);
        let constant750_out1 = self.constant750.val();
        let div42_out1 = linear19_out1
            .clone()
            .div((constant750_out1).unsqueeze_dims(&[0isize, 1isize]));
        let erf5_out1 = div42_out1.erf();
        let constant751_out1 = self.constant751.val();
        let add24_out1 = erf5_out1
            .add((constant751_out1).unsqueeze_dims(&[0isize, 1isize]));
        let mul42_out1 = linear19_out1.mul(add24_out1);
        let constant752_out1 = self.constant752.val();
        let mul43_out1 = mul42_out1
            .mul((constant752_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear20_out1 = self.linear20.forward(mul43_out1);
        let add25_out1 = add23_out1.add(linear20_out1);
        let layernormalization11_out1 = {
            let dtype = add25_out1.clone().dtype();
            self.layernormalization11
                .forward(add25_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let shape51_out1: [i64; 3] = {
            let axes = &layernormalization11_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather42_out1 = shape51_out1[0] as i64;
        let gather43_out1 = shape51_out1[1] as i64;
        let gather44_out1 = shape51_out1[2] as i64;
        let linear21_out1 = self.linear21.forward(layernormalization11_out1);
        let constant756_out1 = 3i64;
        let div43_out1 = gather44_out1 / constant756_out1;
        let unsqueeze59_out1 = [gather42_out1 as i64];
        let unsqueeze60_out1 = [gather43_out1 as i64];
        let unsqueeze61_out1 = [div43_out1 as i64];
        let constant759_out1: [i64; 1] = [3i64];
        let constant760_out1: [i64; 1] = [3i64];
        let concat38_out1: [i64; 5usize] = [
            &unsqueeze59_out1[..],
            &unsqueeze60_out1[..],
            &constant759_out1[..],
            &constant760_out1[..],
            &unsqueeze61_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape17_out1 = linear21_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat38_out1));
        let transpose17_out1 = reshape17_out1.permute([2, 0, 3, 1, 4]);
        let [split6_out1, split6_out2, split6_out3] = {
            let split_tensors = transpose17_out1.split_with_sizes([1, 1, 1].into(), 0);
            split_tensors.try_into().unwrap()
        };
        let squeeze16_out1 = split6_out1.squeeze_dims::<4>(&[0]);
        let squeeze17_out1 = split6_out2.squeeze_dims::<4>(&[0]);
        let squeeze18_out1 = split6_out3.squeeze_dims::<4>(&[0]);
        let slice48_out1 = squeeze16_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice49_out1 = squeeze16_out1.slice(s![.., .., 1.., ..]);
        let slice50_out1 = squeeze17_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice51_out1 = squeeze17_out1.slice(s![.., .., 1.., ..]);
        let mul44_out1 = slice49_out1.clone().mul(unsqueeze8_out1.clone());
        let shape54_out1: [i64; 4] = {
            let axes = &slice49_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather45_out1 = shape54_out1[3] as i64;
        let constant783_out1 = 2i64;
        let div44_out1 = gather45_out1 / constant783_out1;
        let unsqueeze62_out1 = [div44_out1 as i64];
        let slice52_out1 = slice49_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze62_out1[0]]);
        let gather46_out1 = shape54_out1[3] as i64;
        let constant789_out1 = 2i64;
        let div45_out1 = gather46_out1 / constant789_out1;
        let unsqueeze63_out1 = [div45_out1 as i64];
        let slice53_out1 = slice49_out1
            .slice(s![.., .., .., unsqueeze63_out1[0]..9223372036854775807]);
        let neg11_out1 = slice53_out1.neg();
        let concat39_out1 = burn::tensor::Tensor::cat(
            [neg11_out1, slice52_out1].into(),
            3,
        );
        let mul45_out1 = concat39_out1.mul(unsqueeze6_out1.clone());
        let add26_out1 = mul44_out1.add(mul45_out1);
        let mul46_out1 = slice51_out1.clone().mul(unsqueeze8_out1);
        let shape56_out1: [i64; 4] = {
            let axes = &slice51_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather47_out1 = shape56_out1[3] as i64;
        let constant795_out1 = 2i64;
        let div46_out1 = gather47_out1 / constant795_out1;
        let unsqueeze64_out1 = [div46_out1 as i64];
        let slice54_out1 = slice51_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze64_out1[0]]);
        let gather48_out1 = shape56_out1[3] as i64;
        let constant801_out1 = 2i64;
        let div47_out1 = gather48_out1 / constant801_out1;
        let unsqueeze65_out1 = [div47_out1 as i64];
        let slice55_out1 = slice51_out1
            .slice(s![.., .., .., unsqueeze65_out1[0]..9223372036854775807]);
        let neg12_out1 = slice55_out1.neg();
        let concat40_out1 = burn::tensor::Tensor::cat(
            [neg12_out1, slice54_out1].into(),
            3,
        );
        let mul47_out1 = concat40_out1.mul(unsqueeze6_out1);
        let add27_out1 = mul46_out1.add(mul47_out1);
        let concat41_out1 = burn::tensor::Tensor::cat(
            [slice48_out1, add26_out1].into(),
            2,
        );
        let concat42_out1 = burn::tensor::Tensor::cat(
            [slice50_out1, add27_out1].into(),
            2,
        );
        let shape58_out1: [i64; 4] = {
            let axes = &concat41_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice56_out1: [i64; 1] = shape58_out1[3..4].try_into().unwrap();
        let cast83_out1 = {
            let shape_array = slice56_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<
                1,
            >::from_data(
                TensorData::from(float_array),
                (self.device(), burn::tensor::DType::F32),
            )
        };
        let sqrt16_out1 = cast83_out1.sqrt();
        let constant808_out1 = self.constant808.val();
        let div48_out1 = constant808_out1.div(sqrt16_out1);
        let transpose18_out1 = concat42_out1.permute([0, 1, 3, 2]);
        let sqrt17_out1 = div48_out1.sqrt();
        let mul48_out1 = concat41_out1
            .mul((sqrt17_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul49_out1 = transpose18_out1
            .mul((sqrt17_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul32_out1 = mul48_out1.matmul(mul49_out1);
        let softmax6_out1 = burn::tensor::activation::softmax(matmul32_out1, 3);
        let matmul33_out1 = softmax6_out1.matmul(squeeze18_out1);
        let transpose19_out1 = matmul33_out1.permute([0, 2, 1, 3]);
        let unsqueeze66_out1 = [gather42_out1 as i64];
        let unsqueeze67_out1 = [gather43_out1 as i64];
        let unsqueeze68_out1 = [gather44_out1 as i64];
        let concat43_out1: [i64; 3usize] = [
            &unsqueeze66_out1[..],
            &unsqueeze67_out1[..],
            &unsqueeze68_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape18_out1 = transpose19_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat43_out1));
        let linear22_out1 = self.linear22.forward(reshape18_out1);
        let add28_out1 = add25_out1.add(linear22_out1);
        add28_out1
    }
}
#[derive(Module, Debug)]
pub struct Submodule8 {
    layernormalization12: LayerNorm,
    linear23: Linear,
    constant812: burn::module::Param<Tensor<1>>,
    constant813: burn::module::Param<Tensor<1>>,
    constant814: burn::module::Param<Tensor<1>>,
    linear24: Linear,
    layernormalization13: LayerNorm,
    linear25: Linear,
    constant870: burn::module::Param<Tensor<1>>,
    linear26: Linear,
    device: AllocationDevice,
}
impl Submodule8 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization12 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear23 = LinearConfig::new(192, 768).with_bias(true).init(device);
        let constant812: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1.4142135381698608f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant813: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant814: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear24 = LinearConfig::new(768, 192).with_bias(true).init(device);
        let layernormalization13 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear25 = LinearConfig::new(192, 576).with_bias(true).init(device);
        let constant870: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([1], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1].into(),
        );
        let linear26 = LinearConfig::new(192, 192).with_bias(true).init(device);
        Self {
            layernormalization12,
            linear23,
            constant812,
            constant813,
            constant814,
            linear24,
            layernormalization13,
            linear25,
            constant870,
            linear26,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `add28_out1`: expected dtype `DType::F32`
    /// - `unsqueeze8_out1`: expected dtype `DType::F32`
    /// - `unsqueeze6_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        add28_out1: Tensor<3>,
        unsqueeze8_out1: Tensor<4>,
        unsqueeze6_out1: Tensor<4>,
    ) -> Tensor<3> {
        let layernormalization12_out1 = {
            let dtype = add28_out1.clone().dtype();
            self.layernormalization12
                .forward(add28_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear23_out1 = self.linear23.forward(layernormalization12_out1);
        let constant812_out1 = self.constant812.val();
        let div49_out1 = linear23_out1
            .clone()
            .div((constant812_out1).unsqueeze_dims(&[0isize, 1isize]));
        let erf6_out1 = div49_out1.erf();
        let constant813_out1 = self.constant813.val();
        let add29_out1 = erf6_out1
            .add((constant813_out1).unsqueeze_dims(&[0isize, 1isize]));
        let mul50_out1 = linear23_out1.mul(add29_out1);
        let constant814_out1 = self.constant814.val();
        let mul51_out1 = mul50_out1
            .mul((constant814_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear24_out1 = self.linear24.forward(mul51_out1);
        let add30_out1 = add28_out1.add(linear24_out1);
        let layernormalization13_out1 = {
            let dtype = add30_out1.clone().dtype();
            self.layernormalization13
                .forward(add30_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let shape59_out1: [i64; 3] = {
            let axes = &layernormalization13_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather49_out1 = shape59_out1[0] as i64;
        let gather50_out1 = shape59_out1[1] as i64;
        let gather51_out1 = shape59_out1[2] as i64;
        let linear25_out1 = self.linear25.forward(layernormalization13_out1);
        let constant818_out1 = 3i64;
        let div50_out1 = gather51_out1 / constant818_out1;
        let unsqueeze69_out1 = [gather49_out1 as i64];
        let unsqueeze70_out1 = [gather50_out1 as i64];
        let unsqueeze71_out1 = [div50_out1 as i64];
        let constant821_out1: [i64; 1] = [3i64];
        let constant822_out1: [i64; 1] = [3i64];
        let concat44_out1: [i64; 5usize] = [
            &unsqueeze69_out1[..],
            &unsqueeze70_out1[..],
            &constant821_out1[..],
            &constant822_out1[..],
            &unsqueeze71_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape19_out1 = linear25_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat44_out1));
        let transpose20_out1 = reshape19_out1.permute([2, 0, 3, 1, 4]);
        let [split7_out1, split7_out2, split7_out3] = {
            let split_tensors = transpose20_out1.split_with_sizes([1, 1, 1].into(), 0);
            split_tensors.try_into().unwrap()
        };
        let squeeze19_out1 = split7_out1.squeeze_dims::<4>(&[0]);
        let squeeze20_out1 = split7_out2.squeeze_dims::<4>(&[0]);
        let squeeze21_out1 = split7_out3.squeeze_dims::<4>(&[0]);
        let slice57_out1 = squeeze19_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice58_out1 = squeeze19_out1.slice(s![.., .., 1.., ..]);
        let slice59_out1 = squeeze20_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice60_out1 = squeeze20_out1.slice(s![.., .., 1.., ..]);
        let mul52_out1 = slice58_out1.clone().mul(unsqueeze8_out1.clone());
        let shape62_out1: [i64; 4] = {
            let axes = &slice58_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather52_out1 = shape62_out1[3] as i64;
        let constant845_out1 = 2i64;
        let div51_out1 = gather52_out1 / constant845_out1;
        let unsqueeze72_out1 = [div51_out1 as i64];
        let slice61_out1 = slice58_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze72_out1[0]]);
        let gather53_out1 = shape62_out1[3] as i64;
        let constant851_out1 = 2i64;
        let div52_out1 = gather53_out1 / constant851_out1;
        let unsqueeze73_out1 = [div52_out1 as i64];
        let slice62_out1 = slice58_out1
            .slice(s![.., .., .., unsqueeze73_out1[0]..9223372036854775807]);
        let neg13_out1 = slice62_out1.neg();
        let concat45_out1 = burn::tensor::Tensor::cat(
            [neg13_out1, slice61_out1].into(),
            3,
        );
        let mul53_out1 = concat45_out1.mul(unsqueeze6_out1.clone());
        let add31_out1 = mul52_out1.add(mul53_out1);
        let mul54_out1 = slice60_out1.clone().mul(unsqueeze8_out1);
        let shape64_out1: [i64; 4] = {
            let axes = &slice60_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather54_out1 = shape64_out1[3] as i64;
        let constant857_out1 = 2i64;
        let div53_out1 = gather54_out1 / constant857_out1;
        let unsqueeze74_out1 = [div53_out1 as i64];
        let slice63_out1 = slice60_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze74_out1[0]]);
        let gather55_out1 = shape64_out1[3] as i64;
        let constant863_out1 = 2i64;
        let div54_out1 = gather55_out1 / constant863_out1;
        let unsqueeze75_out1 = [div54_out1 as i64];
        let slice64_out1 = slice60_out1
            .slice(s![.., .., .., unsqueeze75_out1[0]..9223372036854775807]);
        let neg14_out1 = slice64_out1.neg();
        let concat46_out1 = burn::tensor::Tensor::cat(
            [neg14_out1, slice63_out1].into(),
            3,
        );
        let mul55_out1 = concat46_out1.mul(unsqueeze6_out1);
        let add32_out1 = mul54_out1.add(mul55_out1);
        let concat47_out1 = burn::tensor::Tensor::cat(
            [slice57_out1, add31_out1].into(),
            2,
        );
        let concat48_out1 = burn::tensor::Tensor::cat(
            [slice59_out1, add32_out1].into(),
            2,
        );
        let shape66_out1: [i64; 4] = {
            let axes = &concat47_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice65_out1: [i64; 1] = shape66_out1[3..4].try_into().unwrap();
        let cast95_out1 = {
            let shape_array = slice65_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<
                1,
            >::from_data(
                TensorData::from(float_array),
                (self.device(), burn::tensor::DType::F32),
            )
        };
        let sqrt19_out1 = cast95_out1.sqrt();
        let constant870_out1 = self.constant870.val();
        let div55_out1 = constant870_out1.div(sqrt19_out1);
        let transpose21_out1 = concat48_out1.permute([0, 1, 3, 2]);
        let sqrt20_out1 = div55_out1.sqrt();
        let mul56_out1 = concat47_out1
            .mul((sqrt20_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul57_out1 = transpose21_out1
            .mul((sqrt20_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul38_out1 = mul56_out1.matmul(mul57_out1);
        let softmax7_out1 = burn::tensor::activation::softmax(matmul38_out1, 3);
        let matmul39_out1 = softmax7_out1.matmul(squeeze21_out1);
        let transpose22_out1 = matmul39_out1.permute([0, 2, 1, 3]);
        let unsqueeze76_out1 = [gather49_out1 as i64];
        let unsqueeze77_out1 = [gather50_out1 as i64];
        let unsqueeze78_out1 = [gather51_out1 as i64];
        let concat49_out1: [i64; 3usize] = [
            &unsqueeze76_out1[..],
            &unsqueeze77_out1[..],
            &unsqueeze78_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape20_out1 = transpose22_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat49_out1));
        let linear26_out1 = self.linear26.forward(reshape20_out1);
        let add33_out1 = add30_out1.add(linear26_out1);
        add33_out1
    }
}
#[derive(Module, Debug)]
pub struct Submodule9 {
    layernormalization14: LayerNorm,
    linear27: Linear,
    constant874: burn::module::Param<Tensor<1>>,
    constant875: burn::module::Param<Tensor<1>>,
    constant876: burn::module::Param<Tensor<1>>,
    linear28: Linear,
    layernormalization15: LayerNorm,
    linear29: Linear,
    constant932: burn::module::Param<Tensor<1>>,
    linear30: Linear,
    device: AllocationDevice,
}
impl Submodule9 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization14 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear27 = LinearConfig::new(192, 768).with_bias(true).init(device);
        let constant874: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1.4142135381698608f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant875: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant876: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear28 = LinearConfig::new(768, 192).with_bias(true).init(device);
        let layernormalization15 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear29 = LinearConfig::new(192, 576).with_bias(true).init(device);
        let constant932: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([1], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1].into(),
        );
        let linear30 = LinearConfig::new(192, 192).with_bias(true).init(device);
        Self {
            layernormalization14,
            linear27,
            constant874,
            constant875,
            constant876,
            linear28,
            layernormalization15,
            linear29,
            constant932,
            linear30,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `add33_out1`: expected dtype `DType::F32`
    /// - `unsqueeze8_out1`: expected dtype `DType::F32`
    /// - `unsqueeze6_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        add33_out1: Tensor<3>,
        unsqueeze8_out1: Tensor<4>,
        unsqueeze6_out1: Tensor<4>,
    ) -> Tensor<3> {
        let layernormalization14_out1 = {
            let dtype = add33_out1.clone().dtype();
            self.layernormalization14
                .forward(add33_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear27_out1 = self.linear27.forward(layernormalization14_out1);
        let constant874_out1 = self.constant874.val();
        let div56_out1 = linear27_out1
            .clone()
            .div((constant874_out1).unsqueeze_dims(&[0isize, 1isize]));
        let erf7_out1 = div56_out1.erf();
        let constant875_out1 = self.constant875.val();
        let add34_out1 = erf7_out1
            .add((constant875_out1).unsqueeze_dims(&[0isize, 1isize]));
        let mul58_out1 = linear27_out1.mul(add34_out1);
        let constant876_out1 = self.constant876.val();
        let mul59_out1 = mul58_out1
            .mul((constant876_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear28_out1 = self.linear28.forward(mul59_out1);
        let add35_out1 = add33_out1.add(linear28_out1);
        let layernormalization15_out1 = {
            let dtype = add35_out1.clone().dtype();
            self.layernormalization15
                .forward(add35_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let shape67_out1: [i64; 3] = {
            let axes = &layernormalization15_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather56_out1 = shape67_out1[0] as i64;
        let gather57_out1 = shape67_out1[1] as i64;
        let gather58_out1 = shape67_out1[2] as i64;
        let linear29_out1 = self.linear29.forward(layernormalization15_out1);
        let constant880_out1 = 3i64;
        let div57_out1 = gather58_out1 / constant880_out1;
        let unsqueeze79_out1 = [gather56_out1 as i64];
        let unsqueeze80_out1 = [gather57_out1 as i64];
        let unsqueeze81_out1 = [div57_out1 as i64];
        let constant883_out1: [i64; 1] = [3i64];
        let constant884_out1: [i64; 1] = [3i64];
        let concat50_out1: [i64; 5usize] = [
            &unsqueeze79_out1[..],
            &unsqueeze80_out1[..],
            &constant883_out1[..],
            &constant884_out1[..],
            &unsqueeze81_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape21_out1 = linear29_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat50_out1));
        let transpose23_out1 = reshape21_out1.permute([2, 0, 3, 1, 4]);
        let [split8_out1, split8_out2, split8_out3] = {
            let split_tensors = transpose23_out1.split_with_sizes([1, 1, 1].into(), 0);
            split_tensors.try_into().unwrap()
        };
        let squeeze22_out1 = split8_out1.squeeze_dims::<4>(&[0]);
        let squeeze23_out1 = split8_out2.squeeze_dims::<4>(&[0]);
        let squeeze24_out1 = split8_out3.squeeze_dims::<4>(&[0]);
        let slice66_out1 = squeeze22_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice67_out1 = squeeze22_out1.slice(s![.., .., 1.., ..]);
        let slice68_out1 = squeeze23_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice69_out1 = squeeze23_out1.slice(s![.., .., 1.., ..]);
        let mul60_out1 = slice67_out1.clone().mul(unsqueeze8_out1.clone());
        let shape70_out1: [i64; 4] = {
            let axes = &slice67_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather59_out1 = shape70_out1[3] as i64;
        let constant907_out1 = 2i64;
        let div58_out1 = gather59_out1 / constant907_out1;
        let unsqueeze82_out1 = [div58_out1 as i64];
        let slice70_out1 = slice67_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze82_out1[0]]);
        let gather60_out1 = shape70_out1[3] as i64;
        let constant913_out1 = 2i64;
        let div59_out1 = gather60_out1 / constant913_out1;
        let unsqueeze83_out1 = [div59_out1 as i64];
        let slice71_out1 = slice67_out1
            .slice(s![.., .., .., unsqueeze83_out1[0]..9223372036854775807]);
        let neg15_out1 = slice71_out1.neg();
        let concat51_out1 = burn::tensor::Tensor::cat(
            [neg15_out1, slice70_out1].into(),
            3,
        );
        let mul61_out1 = concat51_out1.mul(unsqueeze6_out1.clone());
        let add36_out1 = mul60_out1.add(mul61_out1);
        let mul62_out1 = slice69_out1.clone().mul(unsqueeze8_out1);
        let shape72_out1: [i64; 4] = {
            let axes = &slice69_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather61_out1 = shape72_out1[3] as i64;
        let constant919_out1 = 2i64;
        let div60_out1 = gather61_out1 / constant919_out1;
        let unsqueeze84_out1 = [div60_out1 as i64];
        let slice72_out1 = slice69_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze84_out1[0]]);
        let gather62_out1 = shape72_out1[3] as i64;
        let constant925_out1 = 2i64;
        let div61_out1 = gather62_out1 / constant925_out1;
        let unsqueeze85_out1 = [div61_out1 as i64];
        let slice73_out1 = slice69_out1
            .slice(s![.., .., .., unsqueeze85_out1[0]..9223372036854775807]);
        let neg16_out1 = slice73_out1.neg();
        let concat52_out1 = burn::tensor::Tensor::cat(
            [neg16_out1, slice72_out1].into(),
            3,
        );
        let mul63_out1 = concat52_out1.mul(unsqueeze6_out1);
        let add37_out1 = mul62_out1.add(mul63_out1);
        let concat53_out1 = burn::tensor::Tensor::cat(
            [slice66_out1, add36_out1].into(),
            2,
        );
        let concat54_out1 = burn::tensor::Tensor::cat(
            [slice68_out1, add37_out1].into(),
            2,
        );
        let shape74_out1: [i64; 4] = {
            let axes = &concat53_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice74_out1: [i64; 1] = shape74_out1[3..4].try_into().unwrap();
        let cast107_out1 = {
            let shape_array = slice74_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<
                1,
            >::from_data(
                TensorData::from(float_array),
                (self.device(), burn::tensor::DType::F32),
            )
        };
        let sqrt22_out1 = cast107_out1.sqrt();
        let constant932_out1 = self.constant932.val();
        let div62_out1 = constant932_out1.div(sqrt22_out1);
        let transpose24_out1 = concat54_out1.permute([0, 1, 3, 2]);
        let sqrt23_out1 = div62_out1.sqrt();
        let mul64_out1 = concat53_out1
            .mul((sqrt23_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul65_out1 = transpose24_out1
            .mul((sqrt23_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul44_out1 = mul64_out1.matmul(mul65_out1);
        let softmax8_out1 = burn::tensor::activation::softmax(matmul44_out1, 3);
        let matmul45_out1 = softmax8_out1.matmul(squeeze24_out1);
        let transpose25_out1 = matmul45_out1.permute([0, 2, 1, 3]);
        let unsqueeze86_out1 = [gather56_out1 as i64];
        let unsqueeze87_out1 = [gather57_out1 as i64];
        let unsqueeze88_out1 = [gather58_out1 as i64];
        let concat55_out1: [i64; 3usize] = [
            &unsqueeze86_out1[..],
            &unsqueeze87_out1[..],
            &unsqueeze88_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape22_out1 = transpose25_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat55_out1));
        let linear30_out1 = self.linear30.forward(reshape22_out1);
        let add38_out1 = add35_out1.add(linear30_out1);
        add38_out1
    }
}
#[derive(Module, Debug)]
pub struct Submodule10 {
    layernormalization16: LayerNorm,
    linear31: Linear,
    constant936: burn::module::Param<Tensor<1>>,
    constant937: burn::module::Param<Tensor<1>>,
    constant938: burn::module::Param<Tensor<1>>,
    linear32: Linear,
    layernormalization17: LayerNorm,
    linear33: Linear,
    constant998: burn::module::Param<Tensor<1>>,
    linear34: Linear,
    device: AllocationDevice,
}
impl Submodule10 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization16 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear31 = LinearConfig::new(192, 768).with_bias(true).init(device);
        let constant936: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1.4142135381698608f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant937: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant938: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear32 = LinearConfig::new(768, 192).with_bias(true).init(device);
        let layernormalization17 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear33 = LinearConfig::new(192, 576).with_bias(true).init(device);
        let constant998: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([1], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1].into(),
        );
        let linear34 = LinearConfig::new(192, 192).with_bias(true).init(device);
        Self {
            layernormalization16,
            linear31,
            constant936,
            constant937,
            constant938,
            linear32,
            layernormalization17,
            linear33,
            constant998,
            linear34,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `add38_out1`: expected dtype `DType::F32`
    /// - `unsqueeze8_out1`: expected dtype `DType::F32`
    /// - `unsqueeze6_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        add38_out1: Tensor<3>,
        unsqueeze8_out1: Tensor<4>,
        unsqueeze6_out1: Tensor<4>,
    ) -> (Tensor<3>, Tensor<3>) {
        let layernormalization16_out1 = {
            let dtype = add38_out1.clone().dtype();
            self.layernormalization16
                .forward(add38_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear31_out1 = self.linear31.forward(layernormalization16_out1);
        let constant936_out1 = self.constant936.val();
        let div63_out1 = linear31_out1
            .clone()
            .div((constant936_out1).unsqueeze_dims(&[0isize, 1isize]));
        let erf8_out1 = div63_out1.erf();
        let constant937_out1 = self.constant937.val();
        let add39_out1 = erf8_out1
            .add((constant937_out1).unsqueeze_dims(&[0isize, 1isize]));
        let mul66_out1 = linear31_out1.mul(add39_out1);
        let constant938_out1 = self.constant938.val();
        let mul67_out1 = mul66_out1
            .mul((constant938_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear32_out1 = self.linear32.forward(mul67_out1);
        let add40_out1 = add38_out1.add(linear32_out1);
        let slice75_out1 = add40_out1.clone().slice(s![.., 1.., ..]);
        let layernormalization17_out1 = {
            let dtype = add40_out1.clone().dtype();
            self.layernormalization17
                .forward(add40_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let shape75_out1: [i64; 3] = {
            let axes = &layernormalization17_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather63_out1 = shape75_out1[0] as i64;
        let gather64_out1 = shape75_out1[1] as i64;
        let gather65_out1 = shape75_out1[2] as i64;
        let linear33_out1 = self.linear33.forward(layernormalization17_out1);
        let constant946_out1 = 3i64;
        let div64_out1 = gather65_out1 / constant946_out1;
        let unsqueeze89_out1 = [gather63_out1 as i64];
        let unsqueeze90_out1 = [gather64_out1 as i64];
        let unsqueeze91_out1 = [div64_out1 as i64];
        let constant949_out1: [i64; 1] = [3i64];
        let constant950_out1: [i64; 1] = [3i64];
        let concat56_out1: [i64; 5usize] = [
            &unsqueeze89_out1[..],
            &unsqueeze90_out1[..],
            &constant949_out1[..],
            &constant950_out1[..],
            &unsqueeze91_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape23_out1 = linear33_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat56_out1));
        let transpose26_out1 = reshape23_out1.permute([2, 0, 3, 1, 4]);
        let [split9_out1, split9_out2, split9_out3] = {
            let split_tensors = transpose26_out1.split_with_sizes([1, 1, 1].into(), 0);
            split_tensors.try_into().unwrap()
        };
        let squeeze25_out1 = split9_out1.squeeze_dims::<4>(&[0]);
        let squeeze26_out1 = split9_out2.squeeze_dims::<4>(&[0]);
        let squeeze27_out1 = split9_out3.squeeze_dims::<4>(&[0]);
        let slice76_out1 = squeeze25_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice77_out1 = squeeze25_out1.slice(s![.., .., 1.., ..]);
        let slice78_out1 = squeeze26_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice79_out1 = squeeze26_out1.slice(s![.., .., 1.., ..]);
        let mul68_out1 = slice77_out1.clone().mul(unsqueeze8_out1.clone());
        let shape78_out1: [i64; 4] = {
            let axes = &slice77_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather66_out1 = shape78_out1[3] as i64;
        let constant973_out1 = 2i64;
        let div65_out1 = gather66_out1 / constant973_out1;
        let unsqueeze92_out1 = [div65_out1 as i64];
        let slice80_out1 = slice77_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze92_out1[0]]);
        let gather67_out1 = shape78_out1[3] as i64;
        let constant979_out1 = 2i64;
        let div66_out1 = gather67_out1 / constant979_out1;
        let unsqueeze93_out1 = [div66_out1 as i64];
        let slice81_out1 = slice77_out1
            .slice(s![.., .., .., unsqueeze93_out1[0]..9223372036854775807]);
        let neg17_out1 = slice81_out1.neg();
        let concat57_out1 = burn::tensor::Tensor::cat(
            [neg17_out1, slice80_out1].into(),
            3,
        );
        let mul69_out1 = concat57_out1.mul(unsqueeze6_out1.clone());
        let add41_out1 = mul68_out1.add(mul69_out1);
        let mul70_out1 = slice79_out1.clone().mul(unsqueeze8_out1);
        let shape80_out1: [i64; 4] = {
            let axes = &slice79_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather68_out1 = shape80_out1[3] as i64;
        let constant985_out1 = 2i64;
        let div67_out1 = gather68_out1 / constant985_out1;
        let unsqueeze94_out1 = [div67_out1 as i64];
        let slice82_out1 = slice79_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze94_out1[0]]);
        let gather69_out1 = shape80_out1[3] as i64;
        let constant991_out1 = 2i64;
        let div68_out1 = gather69_out1 / constant991_out1;
        let unsqueeze95_out1 = [div68_out1 as i64];
        let slice83_out1 = slice79_out1
            .slice(s![.., .., .., unsqueeze95_out1[0]..9223372036854775807]);
        let neg18_out1 = slice83_out1.neg();
        let concat58_out1 = burn::tensor::Tensor::cat(
            [neg18_out1, slice82_out1].into(),
            3,
        );
        let mul71_out1 = concat58_out1.mul(unsqueeze6_out1);
        let add42_out1 = mul70_out1.add(mul71_out1);
        let concat59_out1 = burn::tensor::Tensor::cat(
            [slice76_out1, add41_out1].into(),
            2,
        );
        let concat60_out1 = burn::tensor::Tensor::cat(
            [slice78_out1, add42_out1].into(),
            2,
        );
        let shape82_out1: [i64; 4] = {
            let axes = &concat59_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice84_out1: [i64; 1] = shape82_out1[3..4].try_into().unwrap();
        let cast119_out1 = {
            let shape_array = slice84_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<
                1,
            >::from_data(
                TensorData::from(float_array),
                (self.device(), burn::tensor::DType::F32),
            )
        };
        let sqrt25_out1 = cast119_out1.sqrt();
        let constant998_out1 = self.constant998.val();
        let div69_out1 = constant998_out1.div(sqrt25_out1);
        let transpose27_out1 = concat60_out1.permute([0, 1, 3, 2]);
        let sqrt26_out1 = div69_out1.sqrt();
        let mul72_out1 = concat59_out1
            .mul((sqrt26_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul73_out1 = transpose27_out1
            .mul((sqrt26_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul50_out1 = mul72_out1.matmul(mul73_out1);
        let softmax9_out1 = burn::tensor::activation::softmax(matmul50_out1, 3);
        let matmul51_out1 = softmax9_out1.matmul(squeeze27_out1);
        let transpose28_out1 = matmul51_out1.permute([0, 2, 1, 3]);
        let unsqueeze96_out1 = [gather63_out1 as i64];
        let unsqueeze97_out1 = [gather64_out1 as i64];
        let unsqueeze98_out1 = [gather65_out1 as i64];
        let concat61_out1: [i64; 3usize] = [
            &unsqueeze96_out1[..],
            &unsqueeze97_out1[..],
            &unsqueeze98_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape24_out1 = transpose28_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat61_out1));
        let linear34_out1 = self.linear34.forward(reshape24_out1);
        let add43_out1 = add40_out1.add(linear34_out1);
        (add43_out1, slice75_out1)
    }
}
#[derive(Module, Debug)]
pub struct Submodule11 {
    layernormalization18: LayerNorm,
    linear35: Linear,
    constant1002: burn::module::Param<Tensor<1>>,
    constant1003: burn::module::Param<Tensor<1>>,
    constant1004: burn::module::Param<Tensor<1>>,
    linear36: Linear,
    layernormalization19: LayerNorm,
    linear37: Linear,
    constant1060: burn::module::Param<Tensor<1>>,
    linear38: Linear,
    device: AllocationDevice,
}
impl Submodule11 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization18 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear35 = LinearConfig::new(192, 768).with_bias(true).init(device);
        let constant1002: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1.4142135381698608f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1003: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1004: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear36 = LinearConfig::new(768, 192).with_bias(true).init(device);
        let layernormalization19 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear37 = LinearConfig::new(192, 576).with_bias(true).init(device);
        let constant1060: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([1], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1].into(),
        );
        let linear38 = LinearConfig::new(192, 192).with_bias(true).init(device);
        Self {
            layernormalization18,
            linear35,
            constant1002,
            constant1003,
            constant1004,
            linear36,
            layernormalization19,
            linear37,
            constant1060,
            linear38,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `add43_out1`: expected dtype `DType::F32`
    /// - `unsqueeze8_out1`: expected dtype `DType::F32`
    /// - `unsqueeze6_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        add43_out1: Tensor<3>,
        unsqueeze8_out1: Tensor<4>,
        unsqueeze6_out1: Tensor<4>,
    ) -> Tensor<3> {
        let layernormalization18_out1 = {
            let dtype = add43_out1.clone().dtype();
            self.layernormalization18
                .forward(add43_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear35_out1 = self.linear35.forward(layernormalization18_out1);
        let constant1002_out1 = self.constant1002.val();
        let div70_out1 = linear35_out1
            .clone()
            .div((constant1002_out1).unsqueeze_dims(&[0isize, 1isize]));
        let erf9_out1 = div70_out1.erf();
        let constant1003_out1 = self.constant1003.val();
        let add44_out1 = erf9_out1
            .add((constant1003_out1).unsqueeze_dims(&[0isize, 1isize]));
        let mul74_out1 = linear35_out1.mul(add44_out1);
        let constant1004_out1 = self.constant1004.val();
        let mul75_out1 = mul74_out1
            .mul((constant1004_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear36_out1 = self.linear36.forward(mul75_out1);
        let add45_out1 = add43_out1.add(linear36_out1);
        let layernormalization19_out1 = {
            let dtype = add45_out1.clone().dtype();
            self.layernormalization19
                .forward(add45_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let shape83_out1: [i64; 3] = {
            let axes = &layernormalization19_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather70_out1 = shape83_out1[0] as i64;
        let gather71_out1 = shape83_out1[1] as i64;
        let gather72_out1 = shape83_out1[2] as i64;
        let linear37_out1 = self.linear37.forward(layernormalization19_out1);
        let constant1008_out1 = 3i64;
        let div71_out1 = gather72_out1 / constant1008_out1;
        let unsqueeze99_out1 = [gather70_out1 as i64];
        let unsqueeze100_out1 = [gather71_out1 as i64];
        let unsqueeze101_out1 = [div71_out1 as i64];
        let constant1011_out1: [i64; 1] = [3i64];
        let constant1012_out1: [i64; 1] = [3i64];
        let concat62_out1: [i64; 5usize] = [
            &unsqueeze99_out1[..],
            &unsqueeze100_out1[..],
            &constant1011_out1[..],
            &constant1012_out1[..],
            &unsqueeze101_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape25_out1 = linear37_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat62_out1));
        let transpose29_out1 = reshape25_out1.permute([2, 0, 3, 1, 4]);
        let [split10_out1, split10_out2, split10_out3] = {
            let split_tensors = transpose29_out1.split_with_sizes([1, 1, 1].into(), 0);
            split_tensors.try_into().unwrap()
        };
        let squeeze28_out1 = split10_out1.squeeze_dims::<4>(&[0]);
        let squeeze29_out1 = split10_out2.squeeze_dims::<4>(&[0]);
        let squeeze30_out1 = split10_out3.squeeze_dims::<4>(&[0]);
        let slice85_out1 = squeeze28_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice86_out1 = squeeze28_out1.slice(s![.., .., 1.., ..]);
        let slice87_out1 = squeeze29_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice88_out1 = squeeze29_out1.slice(s![.., .., 1.., ..]);
        let mul76_out1 = slice86_out1.clone().mul(unsqueeze8_out1.clone());
        let shape86_out1: [i64; 4] = {
            let axes = &slice86_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather73_out1 = shape86_out1[3] as i64;
        let constant1035_out1 = 2i64;
        let div72_out1 = gather73_out1 / constant1035_out1;
        let unsqueeze102_out1 = [div72_out1 as i64];
        let slice89_out1 = slice86_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze102_out1[0]]);
        let gather74_out1 = shape86_out1[3] as i64;
        let constant1041_out1 = 2i64;
        let div73_out1 = gather74_out1 / constant1041_out1;
        let unsqueeze103_out1 = [div73_out1 as i64];
        let slice90_out1 = slice86_out1
            .slice(s![.., .., .., unsqueeze103_out1[0]..9223372036854775807]);
        let neg19_out1 = slice90_out1.neg();
        let concat63_out1 = burn::tensor::Tensor::cat(
            [neg19_out1, slice89_out1].into(),
            3,
        );
        let mul77_out1 = concat63_out1.mul(unsqueeze6_out1.clone());
        let add46_out1 = mul76_out1.add(mul77_out1);
        let mul78_out1 = slice88_out1.clone().mul(unsqueeze8_out1);
        let shape88_out1: [i64; 4] = {
            let axes = &slice88_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather75_out1 = shape88_out1[3] as i64;
        let constant1047_out1 = 2i64;
        let div74_out1 = gather75_out1 / constant1047_out1;
        let unsqueeze104_out1 = [div74_out1 as i64];
        let slice91_out1 = slice88_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze104_out1[0]]);
        let gather76_out1 = shape88_out1[3] as i64;
        let constant1053_out1 = 2i64;
        let div75_out1 = gather76_out1 / constant1053_out1;
        let unsqueeze105_out1 = [div75_out1 as i64];
        let slice92_out1 = slice88_out1
            .slice(s![.., .., .., unsqueeze105_out1[0]..9223372036854775807]);
        let neg20_out1 = slice92_out1.neg();
        let concat64_out1 = burn::tensor::Tensor::cat(
            [neg20_out1, slice91_out1].into(),
            3,
        );
        let mul79_out1 = concat64_out1.mul(unsqueeze6_out1);
        let add47_out1 = mul78_out1.add(mul79_out1);
        let concat65_out1 = burn::tensor::Tensor::cat(
            [slice85_out1, add46_out1].into(),
            2,
        );
        let concat66_out1 = burn::tensor::Tensor::cat(
            [slice87_out1, add47_out1].into(),
            2,
        );
        let shape90_out1: [i64; 4] = {
            let axes = &concat65_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice93_out1: [i64; 1] = shape90_out1[3..4].try_into().unwrap();
        let cast131_out1 = {
            let shape_array = slice93_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<
                1,
            >::from_data(
                TensorData::from(float_array),
                (self.device(), burn::tensor::DType::F32),
            )
        };
        let sqrt28_out1 = cast131_out1.sqrt();
        let constant1060_out1 = self.constant1060.val();
        let div76_out1 = constant1060_out1.div(sqrt28_out1);
        let transpose30_out1 = concat66_out1.permute([0, 1, 3, 2]);
        let sqrt29_out1 = div76_out1.sqrt();
        let mul80_out1 = concat65_out1
            .mul((sqrt29_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul81_out1 = transpose30_out1
            .mul((sqrt29_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul56_out1 = mul80_out1.matmul(mul81_out1);
        let softmax10_out1 = burn::tensor::activation::softmax(matmul56_out1, 3);
        let matmul57_out1 = softmax10_out1.matmul(squeeze30_out1);
        let transpose31_out1 = matmul57_out1.permute([0, 2, 1, 3]);
        let unsqueeze106_out1 = [gather70_out1 as i64];
        let unsqueeze107_out1 = [gather71_out1 as i64];
        let unsqueeze108_out1 = [gather72_out1 as i64];
        let concat67_out1: [i64; 3usize] = [
            &unsqueeze106_out1[..],
            &unsqueeze107_out1[..],
            &unsqueeze108_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape26_out1 = transpose31_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat67_out1));
        let linear38_out1 = self.linear38.forward(reshape26_out1);
        let add48_out1 = add45_out1.add(linear38_out1);
        add48_out1
    }
}
#[derive(Module, Debug)]
pub struct Submodule12 {
    layernormalization20: LayerNorm,
    linear39: Linear,
    constant1064: burn::module::Param<Tensor<1>>,
    constant1065: burn::module::Param<Tensor<1>>,
    constant1066: burn::module::Param<Tensor<1>>,
    linear40: Linear,
    layernormalization21: LayerNorm,
    linear41: Linear,
    constant1122: burn::module::Param<Tensor<1>>,
    linear42: Linear,
    layernormalization22: LayerNorm,
    linear43: Linear,
    constant1126: burn::module::Param<Tensor<1>>,
    constant1127: burn::module::Param<Tensor<1>>,
    constant1128: burn::module::Param<Tensor<1>>,
    linear44: Linear,
    layernormalization23: LayerNorm,
    linear45: Linear,
    constant1184: burn::module::Param<Tensor<1>>,
    linear46: Linear,
    layernormalization24: LayerNorm,
    linear47: Linear,
    constant1188: burn::module::Param<Tensor<1>>,
    constant1189: burn::module::Param<Tensor<1>>,
    constant1190: burn::module::Param<Tensor<1>>,
    linear48: Linear,
    device: AllocationDevice,
}
impl Submodule12 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization20 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear39 = LinearConfig::new(192, 768).with_bias(true).init(device);
        let constant1064: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1.4142135381698608f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1065: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1066: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear40 = LinearConfig::new(768, 192).with_bias(true).init(device);
        let layernormalization21 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear41 = LinearConfig::new(192, 576).with_bias(true).init(device);
        let constant1122: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([1], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1].into(),
        );
        let linear42 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let layernormalization22 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear43 = LinearConfig::new(192, 768).with_bias(true).init(device);
        let constant1126: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1.4142135381698608f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1127: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1128: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear44 = LinearConfig::new(768, 192).with_bias(true).init(device);
        let layernormalization23 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear45 = LinearConfig::new(192, 576).with_bias(true).init(device);
        let constant1184: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([1], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1].into(),
        );
        let linear46 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let layernormalization24 = LayerNormConfig::new(192)
            .with_epsilon(0.0000009999999974752427f64)
            .with_bias(true)
            .init(device);
        let linear47 = LinearConfig::new(192, 768).with_bias(true).init(device);
        let constant1188: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1.4142135381698608f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1189: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1190: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear48 = LinearConfig::new(768, 192).with_bias(true).init(device);
        Self {
            layernormalization20,
            linear39,
            constant1064,
            constant1065,
            constant1066,
            linear40,
            layernormalization21,
            linear41,
            constant1122,
            linear42,
            layernormalization22,
            linear43,
            constant1126,
            constant1127,
            constant1128,
            linear44,
            layernormalization23,
            linear45,
            constant1184,
            linear46,
            layernormalization24,
            linear47,
            constant1188,
            constant1189,
            constant1190,
            linear48,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `add48_out1`: expected dtype `DType::F32`
    /// - `unsqueeze8_out1`: expected dtype `DType::F32`
    /// - `unsqueeze6_out1`: expected dtype `DType::F32`
    /// - `slice38_out1`: expected dtype `DType::F32`
    /// - `slice75_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        add48_out1: Tensor<3>,
        unsqueeze8_out1: Tensor<4>,
        unsqueeze6_out1: Tensor<4>,
        slice38_out1: Tensor<3>,
        gather3_out1: i64,
        div1_out1: i64,
        div2_out1: i64,
        slice75_out1: Tensor<3>,
    ) -> (Tensor<4>, Tensor<4>, Tensor<4>) {
        let layernormalization20_out1 = {
            let dtype = add48_out1.clone().dtype();
            self.layernormalization20
                .forward(add48_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear39_out1 = self.linear39.forward(layernormalization20_out1);
        let constant1064_out1 = self.constant1064.val();
        let div77_out1 = linear39_out1
            .clone()
            .div((constant1064_out1).unsqueeze_dims(&[0isize, 1isize]));
        let erf10_out1 = div77_out1.erf();
        let constant1065_out1 = self.constant1065.val();
        let add49_out1 = erf10_out1
            .add((constant1065_out1).unsqueeze_dims(&[0isize, 1isize]));
        let mul82_out1 = linear39_out1.mul(add49_out1);
        let constant1066_out1 = self.constant1066.val();
        let mul83_out1 = mul82_out1
            .mul((constant1066_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear40_out1 = self.linear40.forward(mul83_out1);
        let add50_out1 = add48_out1.add(linear40_out1);
        let layernormalization21_out1 = {
            let dtype = add50_out1.clone().dtype();
            self.layernormalization21
                .forward(add50_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let shape91_out1: [i64; 3] = {
            let axes = &layernormalization21_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather77_out1 = shape91_out1[0] as i64;
        let gather78_out1 = shape91_out1[1] as i64;
        let gather79_out1 = shape91_out1[2] as i64;
        let linear41_out1 = self.linear41.forward(layernormalization21_out1);
        let constant1070_out1 = 3i64;
        let div78_out1 = gather79_out1 / constant1070_out1;
        let unsqueeze109_out1 = [gather77_out1 as i64];
        let unsqueeze110_out1 = [gather78_out1 as i64];
        let unsqueeze111_out1 = [div78_out1 as i64];
        let constant1073_out1: [i64; 1] = [3i64];
        let constant1074_out1: [i64; 1] = [3i64];
        let concat68_out1: [i64; 5usize] = [
            &unsqueeze109_out1[..],
            &unsqueeze110_out1[..],
            &constant1073_out1[..],
            &constant1074_out1[..],
            &unsqueeze111_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape27_out1 = linear41_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat68_out1));
        let transpose32_out1 = reshape27_out1.permute([2, 0, 3, 1, 4]);
        let [split11_out1, split11_out2, split11_out3] = {
            let split_tensors = transpose32_out1.split_with_sizes([1, 1, 1].into(), 0);
            split_tensors.try_into().unwrap()
        };
        let squeeze31_out1 = split11_out1.squeeze_dims::<4>(&[0]);
        let squeeze32_out1 = split11_out2.squeeze_dims::<4>(&[0]);
        let squeeze33_out1 = split11_out3.squeeze_dims::<4>(&[0]);
        let slice94_out1 = squeeze31_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice95_out1 = squeeze31_out1.slice(s![.., .., 1.., ..]);
        let slice96_out1 = squeeze32_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice97_out1 = squeeze32_out1.slice(s![.., .., 1.., ..]);
        let mul84_out1 = slice95_out1.clone().mul(unsqueeze8_out1.clone());
        let shape94_out1: [i64; 4] = {
            let axes = &slice95_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather80_out1 = shape94_out1[3] as i64;
        let constant1097_out1 = 2i64;
        let div79_out1 = gather80_out1 / constant1097_out1;
        let unsqueeze112_out1 = [div79_out1 as i64];
        let slice98_out1 = slice95_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze112_out1[0]]);
        let gather81_out1 = shape94_out1[3] as i64;
        let constant1103_out1 = 2i64;
        let div80_out1 = gather81_out1 / constant1103_out1;
        let unsqueeze113_out1 = [div80_out1 as i64];
        let slice99_out1 = slice95_out1
            .slice(s![.., .., .., unsqueeze113_out1[0]..9223372036854775807]);
        let neg21_out1 = slice99_out1.neg();
        let concat69_out1 = burn::tensor::Tensor::cat(
            [neg21_out1, slice98_out1].into(),
            3,
        );
        let mul85_out1 = concat69_out1.mul(unsqueeze6_out1.clone());
        let add51_out1 = mul84_out1.add(mul85_out1);
        let mul86_out1 = slice97_out1.clone().mul(unsqueeze8_out1.clone());
        let shape96_out1: [i64; 4] = {
            let axes = &slice97_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather82_out1 = shape96_out1[3] as i64;
        let constant1109_out1 = 2i64;
        let div81_out1 = gather82_out1 / constant1109_out1;
        let unsqueeze114_out1 = [div81_out1 as i64];
        let slice100_out1 = slice97_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze114_out1[0]]);
        let gather83_out1 = shape96_out1[3] as i64;
        let constant1115_out1 = 2i64;
        let div82_out1 = gather83_out1 / constant1115_out1;
        let unsqueeze115_out1 = [div82_out1 as i64];
        let slice101_out1 = slice97_out1
            .slice(s![.., .., .., unsqueeze115_out1[0]..9223372036854775807]);
        let neg22_out1 = slice101_out1.neg();
        let concat70_out1 = burn::tensor::Tensor::cat(
            [neg22_out1, slice100_out1].into(),
            3,
        );
        let mul87_out1 = concat70_out1.mul(unsqueeze6_out1.clone());
        let add52_out1 = mul86_out1.add(mul87_out1);
        let concat71_out1 = burn::tensor::Tensor::cat(
            [slice94_out1, add51_out1].into(),
            2,
        );
        let concat72_out1 = burn::tensor::Tensor::cat(
            [slice96_out1, add52_out1].into(),
            2,
        );
        let shape98_out1: [i64; 4] = {
            let axes = &concat71_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice102_out1: [i64; 1] = shape98_out1[3..4].try_into().unwrap();
        let cast143_out1 = {
            let shape_array = slice102_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<
                1,
            >::from_data(
                TensorData::from(float_array),
                (self.device(), burn::tensor::DType::F32),
            )
        };
        let sqrt31_out1 = cast143_out1.sqrt();
        let constant1122_out1 = self.constant1122.val();
        let div83_out1 = constant1122_out1.div(sqrt31_out1);
        let transpose33_out1 = concat72_out1.permute([0, 1, 3, 2]);
        let sqrt32_out1 = div83_out1.sqrt();
        let mul88_out1 = concat71_out1
            .mul((sqrt32_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul89_out1 = transpose33_out1
            .mul((sqrt32_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul62_out1 = mul88_out1.matmul(mul89_out1);
        let softmax11_out1 = burn::tensor::activation::softmax(matmul62_out1, 3);
        let matmul63_out1 = softmax11_out1.matmul(squeeze33_out1);
        let transpose34_out1 = matmul63_out1.permute([0, 2, 1, 3]);
        let unsqueeze116_out1 = [gather77_out1 as i64];
        let unsqueeze117_out1 = [gather78_out1 as i64];
        let unsqueeze118_out1 = [gather79_out1 as i64];
        let concat73_out1: [i64; 3usize] = [
            &unsqueeze116_out1[..],
            &unsqueeze117_out1[..],
            &unsqueeze118_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape28_out1 = transpose34_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat73_out1));
        let linear42_out1 = self.linear42.forward(reshape28_out1);
        let add53_out1 = add50_out1.add(linear42_out1);
        let layernormalization22_out1 = {
            let dtype = add53_out1.clone().dtype();
            self.layernormalization22
                .forward(add53_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear43_out1 = self.linear43.forward(layernormalization22_out1);
        let constant1126_out1 = self.constant1126.val();
        let div84_out1 = linear43_out1
            .clone()
            .div((constant1126_out1).unsqueeze_dims(&[0isize, 1isize]));
        let erf11_out1 = div84_out1.erf();
        let constant1127_out1 = self.constant1127.val();
        let add54_out1 = erf11_out1
            .add((constant1127_out1).unsqueeze_dims(&[0isize, 1isize]));
        let mul90_out1 = linear43_out1.mul(add54_out1);
        let constant1128_out1 = self.constant1128.val();
        let mul91_out1 = mul90_out1
            .mul((constant1128_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear44_out1 = self.linear44.forward(mul91_out1);
        let add55_out1 = add53_out1.add(linear44_out1);
        let layernormalization23_out1 = {
            let dtype = add55_out1.clone().dtype();
            self.layernormalization23
                .forward(add55_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let shape99_out1: [i64; 3] = {
            let axes = &layernormalization23_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather84_out1 = shape99_out1[0] as i64;
        let gather85_out1 = shape99_out1[1] as i64;
        let gather86_out1 = shape99_out1[2] as i64;
        let linear45_out1 = self.linear45.forward(layernormalization23_out1);
        let constant1132_out1 = 3i64;
        let div85_out1 = gather86_out1 / constant1132_out1;
        let unsqueeze119_out1 = [gather84_out1 as i64];
        let unsqueeze120_out1 = [gather85_out1 as i64];
        let unsqueeze121_out1 = [div85_out1 as i64];
        let constant1135_out1: [i64; 1] = [3i64];
        let constant1136_out1: [i64; 1] = [3i64];
        let concat74_out1: [i64; 5usize] = [
            &unsqueeze119_out1[..],
            &unsqueeze120_out1[..],
            &constant1135_out1[..],
            &constant1136_out1[..],
            &unsqueeze121_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape29_out1 = linear45_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat74_out1));
        let transpose35_out1 = reshape29_out1.permute([2, 0, 3, 1, 4]);
        let [split12_out1, split12_out2, split12_out3] = {
            let split_tensors = transpose35_out1.split_with_sizes([1, 1, 1].into(), 0);
            split_tensors.try_into().unwrap()
        };
        let squeeze34_out1 = split12_out1.squeeze_dims::<4>(&[0]);
        let squeeze35_out1 = split12_out2.squeeze_dims::<4>(&[0]);
        let squeeze36_out1 = split12_out3.squeeze_dims::<4>(&[0]);
        let slice103_out1 = squeeze34_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice104_out1 = squeeze34_out1.slice(s![.., .., 1.., ..]);
        let slice105_out1 = squeeze35_out1.clone().slice(s![.., .., 0..1, ..]);
        let slice106_out1 = squeeze35_out1.slice(s![.., .., 1.., ..]);
        let mul92_out1 = slice104_out1.clone().mul(unsqueeze8_out1.clone());
        let shape102_out1: [i64; 4] = {
            let axes = &slice104_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather87_out1 = shape102_out1[3] as i64;
        let constant1159_out1 = 2i64;
        let div86_out1 = gather87_out1 / constant1159_out1;
        let unsqueeze122_out1 = [div86_out1 as i64];
        let slice107_out1 = slice104_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze122_out1[0]]);
        let gather88_out1 = shape102_out1[3] as i64;
        let constant1165_out1 = 2i64;
        let div87_out1 = gather88_out1 / constant1165_out1;
        let unsqueeze123_out1 = [div87_out1 as i64];
        let slice108_out1 = slice104_out1
            .slice(s![.., .., .., unsqueeze123_out1[0]..9223372036854775807]);
        let neg23_out1 = slice108_out1.neg();
        let concat75_out1 = burn::tensor::Tensor::cat(
            [neg23_out1, slice107_out1].into(),
            3,
        );
        let mul93_out1 = concat75_out1.mul(unsqueeze6_out1.clone());
        let add56_out1 = mul92_out1.add(mul93_out1);
        let mul94_out1 = slice106_out1.clone().mul(unsqueeze8_out1);
        let shape104_out1: [i64; 4] = {
            let axes = &slice106_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather89_out1 = shape104_out1[3] as i64;
        let constant1171_out1 = 2i64;
        let div88_out1 = gather89_out1 / constant1171_out1;
        let unsqueeze124_out1 = [div88_out1 as i64];
        let slice109_out1 = slice106_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze124_out1[0]]);
        let gather90_out1 = shape104_out1[3] as i64;
        let constant1177_out1 = 2i64;
        let div89_out1 = gather90_out1 / constant1177_out1;
        let unsqueeze125_out1 = [div89_out1 as i64];
        let slice110_out1 = slice106_out1
            .slice(s![.., .., .., unsqueeze125_out1[0]..9223372036854775807]);
        let neg24_out1 = slice110_out1.neg();
        let concat76_out1 = burn::tensor::Tensor::cat(
            [neg24_out1, slice109_out1].into(),
            3,
        );
        let mul95_out1 = concat76_out1.mul(unsqueeze6_out1);
        let add57_out1 = mul94_out1.add(mul95_out1);
        let concat77_out1 = burn::tensor::Tensor::cat(
            [slice103_out1, add56_out1].into(),
            2,
        );
        let concat78_out1 = burn::tensor::Tensor::cat(
            [slice105_out1, add57_out1].into(),
            2,
        );
        let shape106_out1: [i64; 4] = {
            let axes = &concat77_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice111_out1: [i64; 1] = shape106_out1[3..4].try_into().unwrap();
        let cast155_out1 = {
            let shape_array = slice111_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<
                1,
            >::from_data(
                TensorData::from(float_array),
                (self.device(), burn::tensor::DType::F32),
            )
        };
        let sqrt34_out1 = cast155_out1.sqrt();
        let constant1184_out1 = self.constant1184.val();
        let div90_out1 = constant1184_out1.div(sqrt34_out1);
        let transpose36_out1 = concat78_out1.permute([0, 1, 3, 2]);
        let sqrt35_out1 = div90_out1.sqrt();
        let mul96_out1 = concat77_out1
            .mul((sqrt35_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul97_out1 = transpose36_out1
            .mul((sqrt35_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul68_out1 = mul96_out1.matmul(mul97_out1);
        let softmax12_out1 = burn::tensor::activation::softmax(matmul68_out1, 3);
        let matmul69_out1 = softmax12_out1.matmul(squeeze36_out1);
        let transpose37_out1 = matmul69_out1.permute([0, 2, 1, 3]);
        let unsqueeze126_out1 = [gather84_out1 as i64];
        let unsqueeze127_out1 = [gather85_out1 as i64];
        let unsqueeze128_out1 = [gather86_out1 as i64];
        let concat79_out1: [i64; 3usize] = [
            &unsqueeze126_out1[..],
            &unsqueeze127_out1[..],
            &unsqueeze128_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape30_out1 = transpose37_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat79_out1));
        let linear46_out1 = self.linear46.forward(reshape30_out1);
        let add58_out1 = add55_out1.add(linear46_out1);
        let layernormalization24_out1 = {
            let dtype = add58_out1.clone().dtype();
            self.layernormalization24
                .forward(add58_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear47_out1 = self.linear47.forward(layernormalization24_out1);
        let constant1188_out1 = self.constant1188.val();
        let div91_out1 = linear47_out1
            .clone()
            .div((constant1188_out1).unsqueeze_dims(&[0isize, 1isize]));
        let erf12_out1 = div91_out1.erf();
        let constant1189_out1 = self.constant1189.val();
        let add59_out1 = erf12_out1
            .add((constant1189_out1).unsqueeze_dims(&[0isize, 1isize]));
        let mul98_out1 = linear47_out1.mul(add59_out1);
        let constant1190_out1 = self.constant1190.val();
        let mul99_out1 = mul98_out1
            .mul((constant1190_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear48_out1 = self.linear48.forward(mul99_out1);
        let add60_out1 = add58_out1.add(linear48_out1);
        let slice112_out1 = add60_out1.slice(s![.., 1.., ..]);
        let transpose38_out1 = slice38_out1.permute([0, 2, 1]);
        let unsqueeze129_out1 = [gather3_out1 as i64];
        let unsqueeze130_out1 = [div1_out1 as i64];
        let unsqueeze131_out1 = [div2_out1 as i64];
        let constant1196_out1: [i64; 1] = [-1i64];
        let concat80_out1: [i64; 4usize] = [
            &unsqueeze129_out1[..],
            &constant1196_out1[..],
            &unsqueeze130_out1[..],
            &unsqueeze131_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let unsqueeze132_out1 = [gather3_out1 as i64];
        let unsqueeze133_out1 = [div1_out1 as i64];
        let unsqueeze134_out1 = [div2_out1 as i64];
        let constant1200_out1: [i64; 1] = [-1i64];
        let concat81_out1: [i64; 4usize] = [
            &unsqueeze132_out1[..],
            &constant1200_out1[..],
            &unsqueeze133_out1[..],
            &unsqueeze134_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let unsqueeze135_out1 = [gather3_out1 as i64];
        let unsqueeze136_out1 = [div1_out1 as i64];
        let unsqueeze137_out1 = [div2_out1 as i64];
        let constant1204_out1: [i64; 1] = [-1i64];
        let concat82_out1: [i64; 4usize] = [
            &unsqueeze135_out1[..],
            &constant1204_out1[..],
            &unsqueeze136_out1[..],
            &unsqueeze137_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape31_out1 = transpose38_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat80_out1));
        let shape107_out1: [i64; 4] = {
            let axes = &reshape31_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice113_out1: [i64; 2] = shape107_out1[0..2].try_into().unwrap();
        let constant1210_out1: [i64; 2] = [100i64, 100i64];
        let concat83_out1: [i64; 4usize] = [&slice113_out1[..], &constant1210_out1[..]]
            .concat()
            .try_into()
            .unwrap();
        let resize1_out1 = {
            let target_height = concat83_out1[2] as usize;
            let target_width = concat83_out1[3] as usize;
            burn::tensor::module::interpolate(
                reshape31_out1,
                burn::tensor::ops::InterpolateOptions::new(
                        (burn::nn::interpolate::InterpolateMode::Linear {
                            coordinate_transform: burn::tensor::ops::CoordinateTransformMode::HalfPixel,
                        })
                            .into(),
                    )
                    .with_output_size([target_height, target_width]),
            )
        };
        let transpose39_out1 = slice75_out1.permute([0, 2, 1]);
        let reshape32_out1 = transpose39_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat81_out1));
        let shape108_out1: [i64; 4] = {
            let axes = &reshape32_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice114_out1: [i64; 2] = shape108_out1[0..2].try_into().unwrap();
        let constant1214_out1: [i64; 2] = [50i64, 50i64];
        let concat84_out1: [i64; 4usize] = [&slice114_out1[..], &constant1214_out1[..]]
            .concat()
            .try_into()
            .unwrap();
        let resize2_out1 = {
            let target_height = concat84_out1[2] as usize;
            let target_width = concat84_out1[3] as usize;
            burn::tensor::module::interpolate(
                reshape32_out1,
                burn::tensor::ops::InterpolateOptions::new(
                        (burn::nn::interpolate::InterpolateMode::Linear {
                            coordinate_transform: burn::tensor::ops::CoordinateTransformMode::HalfPixel,
                        })
                            .into(),
                    )
                    .with_output_size([target_height, target_width]),
            )
        };
        let transpose40_out1 = slice112_out1.permute([0, 2, 1]);
        let reshape33_out1 = transpose40_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat82_out1));
        (reshape33_out1, resize1_out1, resize2_out1)
    }
}
#[derive(Module, Debug)]
pub struct Submodule13 {
    conv2d2: Conv2d,
    constant1219: burn::module::Param<Tensor<1>>,
    constant1220: burn::module::Param<Tensor<1>>,
    constant1221: burn::module::Param<Tensor<1>>,
    maxpool2d1: MaxPool2d,
    conv2d3: Conv2d,
    constant1222: burn::module::Param<Tensor<1>>,
    constant1223: burn::module::Param<Tensor<1>>,
    constant1224: burn::module::Param<Tensor<1>>,
    conv2d4: Conv2d,
    constant1225: burn::module::Param<Tensor<1>>,
    constant1226: burn::module::Param<Tensor<1>>,
    constant1227: burn::module::Param<Tensor<1>>,
    conv2d5: Conv2d,
    conv2d6: Conv2d,
    conv2d7: Conv2d,
    conv2d8: Conv2d,
    constant1234: burn::module::Param<Tensor<3>>,
    linear49: Linear,
    linear50: Linear,
    linear51: Linear,
    constant1251: burn::module::Param<Tensor<1>>,
    linear52: Linear,
    layernormalization25: LayerNorm,
    linear53: Linear,
    constant1258: burn::module::Param<Tensor<1>>,
    constant1259: burn::module::Param<Tensor<1>>,
    constant1260: burn::module::Param<Tensor<1>>,
    linear54: Linear,
    layernormalization26: LayerNorm,
    device: AllocationDevice,
}
impl Submodule13 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let conv2d2 = Conv2dConfig::new([3, 16], [3, 3])
            .with_stride([2, 2])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let constant1219: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1.4142135381698608f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1220: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1221: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let maxpool2d1 = MaxPool2dConfig::new([3, 3])
            .with_strides([2, 2])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_ceil_mode(false)
            .init();
        let conv2d3 = Conv2dConfig::new([16, 32], [3, 3])
            .with_stride([2, 2])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let constant1222: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1.4142135381698608f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1223: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1224: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let conv2d4 = Conv2dConfig::new([32, 64], [3, 3])
            .with_stride([2, 2])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let constant1225: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1.4142135381698608f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1226: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1227: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let conv2d5 = Conv2dConfig::new([64, 64], [3, 3])
            .with_stride([2, 2])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d6 = Conv2dConfig::new([224, 192], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d7 = Conv2dConfig::new([256, 192], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d8 = Conv2dConfig::new([256, 192], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let constant1234: burn::module::Param<Tensor<3>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                3,
            >::zeros([1, 625, 192], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1, 625, 192].into(),
        );
        let linear49 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear50 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear51 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let constant1251: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.20412415266036987f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear52 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let layernormalization25 = LayerNormConfig::new(192)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear53 = LinearConfig::new(192, 512).with_bias(true).init(device);
        let constant1258: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1.4142135381698608f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1259: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1260: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear54 = LinearConfig::new(512, 192).with_bias(true).init(device);
        let layernormalization26 = LayerNormConfig::new(192)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        Self {
            conv2d2,
            constant1219,
            constant1220,
            constant1221,
            maxpool2d1,
            conv2d3,
            constant1222,
            constant1223,
            constant1224,
            conv2d4,
            constant1225,
            constant1226,
            constant1227,
            conv2d5,
            conv2d6,
            conv2d7,
            conv2d8,
            constant1234,
            linear49,
            linear50,
            linear51,
            constant1251,
            linear52,
            layernormalization25,
            linear53,
            constant1258,
            constant1259,
            constant1260,
            linear54,
            layernormalization26,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `reshape33_out1`: expected dtype `DType::F32`
    /// - `images`: expected dtype `DType::F32`
    /// - `resize1_out1`: expected dtype `DType::F32`
    /// - `resize2_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        reshape33_out1: Tensor<4>,
        images: Tensor<4>,
        resize1_out1: Tensor<4>,
        resize2_out1: Tensor<4>,
    ) -> (Tensor<4>, Tensor<4>, Tensor<4>) {
        let shape109_out1: [i64; 4] = {
            let axes = &reshape33_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice115_out1: [i64; 2] = shape109_out1[0..2].try_into().unwrap();
        let constant1218_out1: [i64; 2] = [25i64, 25i64];
        let concat85_out1: [i64; 4usize] = [&slice115_out1[..], &constant1218_out1[..]]
            .concat()
            .try_into()
            .unwrap();
        let resize3_out1 = {
            let target_height = concat85_out1[2] as usize;
            let target_width = concat85_out1[3] as usize;
            burn::tensor::module::interpolate(
                reshape33_out1,
                burn::tensor::ops::InterpolateOptions::new(
                        (burn::nn::interpolate::InterpolateMode::Linear {
                            coordinate_transform: burn::tensor::ops::CoordinateTransformMode::HalfPixel,
                        })
                            .into(),
                    )
                    .with_output_size([target_height, target_width]),
            )
        };
        let conv2d2_out1 = self.conv2d2.forward(images);
        let constant1219_out1 = self.constant1219.val();
        let div92_out1 = conv2d2_out1
            .clone()
            .div((constant1219_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let erf13_out1 = div92_out1.erf();
        let constant1220_out1 = self.constant1220.val();
        let add61_out1 = erf13_out1
            .add((constant1220_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul100_out1 = conv2d2_out1.mul(add61_out1);
        let constant1221_out1 = self.constant1221.val();
        let mul101_out1 = mul100_out1
            .mul((constant1221_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let maxpool2d1_out1 = self.maxpool2d1.forward(mul101_out1);
        let conv2d3_out1 = self.conv2d3.forward(maxpool2d1_out1);
        let constant1222_out1 = self.constant1222.val();
        let div93_out1 = conv2d3_out1
            .clone()
            .div((constant1222_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let erf14_out1 = div93_out1.erf();
        let constant1223_out1 = self.constant1223.val();
        let add62_out1 = erf14_out1
            .add((constant1223_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul102_out1 = conv2d3_out1.clone().mul(add62_out1);
        let constant1224_out1 = self.constant1224.val();
        let mul103_out1 = mul102_out1
            .mul((constant1224_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let conv2d4_out1 = self.conv2d4.forward(mul103_out1);
        let constant1225_out1 = self.constant1225.val();
        let div94_out1 = conv2d4_out1
            .clone()
            .div((constant1225_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let erf15_out1 = div94_out1.erf();
        let constant1226_out1 = self.constant1226.val();
        let add63_out1 = erf15_out1
            .add((constant1226_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul104_out1 = conv2d4_out1.clone().mul(add63_out1);
        let constant1227_out1 = self.constant1227.val();
        let mul105_out1 = mul104_out1
            .mul((constant1227_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let conv2d5_out1 = self.conv2d5.forward(mul105_out1);
        let concat86_out1 = burn::tensor::Tensor::cat(
            [resize1_out1, conv2d3_out1].into(),
            1,
        );
        let concat87_out1 = burn::tensor::Tensor::cat(
            [resize2_out1, conv2d4_out1].into(),
            1,
        );
        let concat88_out1 = burn::tensor::Tensor::cat(
            [resize3_out1, conv2d5_out1].into(),
            1,
        );
        let conv2d6_out1 = self.conv2d6.forward(concat86_out1);
        let conv2d7_out1 = self.conv2d7.forward(concat87_out1);
        let conv2d8_out1 = self.conv2d8.forward(concat88_out1);
        let shape110_out1: [i64; 4] = {
            let axes = &conv2d8_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather91_out1 = shape110_out1[2] as i64;
        let gather92_out1 = shape110_out1[3] as i64;
        let slice116_out1: [i64; 2] = shape110_out1[0..2].try_into().unwrap();
        let constant1233_out1: [i64; 1] = [-1i64];
        let concat89_out1: [i64; 3usize] = [&slice116_out1[..], &constant1233_out1[..]]
            .concat()
            .try_into()
            .unwrap();
        let reshape34_out1 = conv2d8_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat89_out1));
        let transpose41_out1 = reshape34_out1.clone().permute([0, 2, 1]);
        let constant1234_out1 = self.constant1234.val();
        let add64_out1 = transpose41_out1.clone().add(constant1234_out1);
        let transpose42_out1 = add64_out1.permute([1, 0, 2]);
        let transpose43_out1 = reshape34_out1.permute([2, 0, 1]);
        let shape113_out1: [i64; 3] = {
            let axes = &transpose42_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather93_out1 = shape113_out1[0] as i64;
        let gather94_out1 = shape113_out1[1] as i64;
        let gather95_out1 = shape113_out1[2] as i64;
        let constant1238_out1 = 8i64;
        let div95_out1 = gather95_out1 / constant1238_out1;
        let linear49_out1 = self.linear49.forward(transpose42_out1.clone());
        let linear50_out1 = self.linear50.forward(transpose42_out1);
        let linear51_out1 = self.linear51.forward(transpose43_out1);
        let constant1239_out1 = 8i64;
        let mul106_out1 = gather94_out1 * constant1239_out1;
        let unsqueeze138_out1 = [gather93_out1 as i64];
        let unsqueeze139_out1 = [mul106_out1 as i64];
        let unsqueeze140_out1 = [div95_out1 as i64];
        let concat90_out1: [i64; 3usize] = [
            &unsqueeze138_out1[..],
            &unsqueeze139_out1[..],
            &unsqueeze140_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape35_out1 = linear49_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat90_out1));
        let transpose44_out1 = reshape35_out1.permute([1, 0, 2]);
        let shape116_out1: [i64; 3] = {
            let axes = &linear50_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather96_out1 = shape116_out1[0] as i64;
        let unsqueeze141_out1 = [gather96_out1 as i64];
        let unsqueeze142_out1 = [mul106_out1 as i64];
        let unsqueeze143_out1 = [div95_out1 as i64];
        let concat91_out1: [i64; 3usize] = [
            &unsqueeze141_out1[..],
            &unsqueeze142_out1[..],
            &unsqueeze143_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape36_out1 = linear50_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat91_out1));
        let shape117_out1: [i64; 3] = {
            let axes = &linear51_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather97_out1 = shape117_out1[0] as i64;
        let unsqueeze144_out1 = [gather97_out1 as i64];
        let unsqueeze145_out1 = [mul106_out1 as i64];
        let unsqueeze146_out1 = [div95_out1 as i64];
        let concat92_out1: [i64; 3usize] = [
            &unsqueeze144_out1[..],
            &unsqueeze145_out1[..],
            &unsqueeze146_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape37_out1 = linear51_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat92_out1));
        let transpose45_out1 = reshape37_out1.permute([1, 0, 2]);
        let constant1251_out1 = self.constant1251.val();
        let mul107_out1 = transpose44_out1
            .mul((constant1251_out1).unsqueeze_dims(&[0isize, 1isize]));
        let transpose46_out1 = reshape36_out1.permute([1, 2, 0]);
        let matmul76_out1 = mul107_out1.matmul(transpose46_out1);
        let softmax13_out1 = burn::tensor::activation::softmax(matmul76_out1, 2);
        let matmul77_out1 = softmax13_out1.matmul(transpose45_out1);
        let transpose47_out1 = matmul77_out1.permute([1, 0, 2]);
        let mul108_out1 = gather93_out1 * gather94_out1;
        let unsqueeze147_out1 = [mul108_out1 as i64];
        let unsqueeze148_out1 = [gather95_out1 as i64];
        let concat93_out1: [i64; 2usize] = [
            &unsqueeze147_out1[..],
            &unsqueeze148_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape38_out1 = transpose47_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat93_out1));
        let linear52_out1 = self.linear52.forward(reshape38_out1);
        let shape118_out1: [i64; 2] = {
            let axes = &linear52_out1.clone().dims()[0..2];
            let mut output = [0i64; 2];
            for i in 0..2 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather98_out1 = shape118_out1[1] as i64;
        let unsqueeze149_out1 = [gather93_out1 as i64];
        let unsqueeze150_out1 = [gather94_out1 as i64];
        let unsqueeze151_out1 = [gather98_out1 as i64];
        let concat94_out1: [i64; 3usize] = [
            &unsqueeze149_out1[..],
            &unsqueeze150_out1[..],
            &unsqueeze151_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape39_out1 = linear52_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat94_out1));
        let transpose48_out1 = reshape39_out1.permute([1, 0, 2]);
        let add65_out1 = transpose41_out1.add(transpose48_out1);
        let layernormalization25_out1 = {
            let dtype = add65_out1.dtype();
            self.layernormalization25
                .forward(add65_out1.cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear53_out1 = self.linear53.forward(layernormalization25_out1.clone());
        let constant1258_out1 = self.constant1258.val();
        let div96_out1 = linear53_out1
            .clone()
            .div((constant1258_out1).unsqueeze_dims(&[0isize, 1isize]));
        let erf16_out1 = div96_out1.erf();
        let constant1259_out1 = self.constant1259.val();
        let add66_out1 = erf16_out1
            .add((constant1259_out1).unsqueeze_dims(&[0isize, 1isize]));
        let mul109_out1 = linear53_out1.mul(add66_out1);
        let constant1260_out1 = self.constant1260.val();
        let mul110_out1 = mul109_out1
            .mul((constant1260_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear54_out1 = self.linear54.forward(mul110_out1);
        let add67_out1 = layernormalization25_out1.add(linear54_out1);
        let layernormalization26_out1 = {
            let dtype = add67_out1.dtype();
            self.layernormalization26
                .forward(add67_out1.cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let transpose49_out1 = layernormalization26_out1.permute([0, 2, 1]);
        let unsqueeze152_out1 = [gather91_out1 as i64];
        let unsqueeze153_out1 = [gather92_out1 as i64];
        let constant1261_out1: [i64; 1] = [-1i64];
        let constant1262_out1: [i64; 1] = [192i64];
        let concat95_out1: [i64; 4usize] = [
            &constant1261_out1[..],
            &constant1262_out1[..],
            &unsqueeze152_out1[..],
            &unsqueeze153_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape40_out1 = transpose49_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat95_out1));
        (reshape40_out1, conv2d7_out1, conv2d6_out1)
    }
}
#[derive(Module, Debug)]
pub struct Submodule14 {
    conv2d9: Conv2d,
    resize4: burn::nn::interpolate::Interpolate2d,
    conv2d10: Conv2d,
    conv2d11: Conv2d,
    conv2d12: Conv2d,
    conv2d13: Conv2d,
    conv2d14: Conv2d,
    conv2d15: Conv2d,
    conv2d16: Conv2d,
    conv2d17: Conv2d,
    conv2d18: Conv2d,
    resize5: burn::nn::interpolate::Interpolate2d,
    conv2d19: Conv2d,
    conv2d20: Conv2d,
    conv2d21: Conv2d,
    conv2d22: Conv2d,
    conv2d23: Conv2d,
    conv2d24: Conv2d,
    conv2d25: Conv2d,
    device: AllocationDevice,
}
impl Submodule14 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let conv2d9 = Conv2dConfig::new([192, 192], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let resize4 = burn::nn::interpolate::Interpolate2dConfig::new()
            .with_output_size(None)
            .with_scale_factor(Some([2.0, 2.0]))
            .with_mode(burn::nn::interpolate::InterpolateMode::Nearest)
            .init();
        let conv2d10 = Conv2dConfig::new([192, 384], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d11 = Conv2dConfig::new([192, 64], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d12 = Conv2dConfig::new([32, 32], [3, 3])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d13 = Conv2dConfig::new([32, 32], [3, 3])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d14 = Conv2dConfig::new([32, 64], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d15 = Conv2dConfig::new([32, 32], [3, 3])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d16 = Conv2dConfig::new([32, 32], [3, 3])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d17 = Conv2dConfig::new([448, 192], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d18 = Conv2dConfig::new([192, 192], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let resize5 = burn::nn::interpolate::Interpolate2dConfig::new()
            .with_output_size(None)
            .with_scale_factor(Some([2.0, 2.0]))
            .with_mode(burn::nn::interpolate::InterpolateMode::Nearest)
            .init();
        let conv2d19 = Conv2dConfig::new([192, 384], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d20 = Conv2dConfig::new([192, 64], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d21 = Conv2dConfig::new([32, 32], [3, 3])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d22 = Conv2dConfig::new([32, 32], [3, 3])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d23 = Conv2dConfig::new([32, 64], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d24 = Conv2dConfig::new([32, 32], [3, 3])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d25 = Conv2dConfig::new([32, 32], [3, 3])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        Self {
            conv2d9,
            resize4,
            conv2d10,
            conv2d11,
            conv2d12,
            conv2d13,
            conv2d14,
            conv2d15,
            conv2d16,
            conv2d17,
            conv2d18,
            resize5,
            conv2d19,
            conv2d20,
            conv2d21,
            conv2d22,
            conv2d23,
            conv2d24,
            conv2d25,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `reshape40_out1`: expected dtype `DType::F32`
    /// - `conv2d7_out1`: expected dtype `DType::F32`
    /// - `conv2d6_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        reshape40_out1: Tensor<4>,
        conv2d7_out1: Tensor<4>,
        conv2d6_out1: Tensor<4>,
    ) -> (Tensor<4>, Tensor<4>, Tensor<4>) {
        let conv2d9_out1 = self.conv2d9.forward(reshape40_out1);
        let resize4_out1 = self.resize4.forward(conv2d9_out1.clone());
        let add68_out1 = resize4_out1.add(conv2d7_out1);
        let conv2d10_out1 = self.conv2d10.forward(add68_out1);
        let sigmoid1_out1 = burn::tensor::activation::sigmoid(conv2d10_out1.clone());
        let mul111_out1 = conv2d10_out1.mul(sigmoid1_out1);
        let [split13_out1, split13_out2] = {
            let split_tensors = mul111_out1.split_with_sizes([192, 192].into(), 1);
            split_tensors.try_into().unwrap()
        };
        let conv2d11_out1 = self.conv2d11.forward(split13_out2.clone());
        let sigmoid2_out1 = burn::tensor::activation::sigmoid(conv2d11_out1.clone());
        let mul112_out1 = conv2d11_out1.mul(sigmoid2_out1);
        let shape119_out1: [i64; 4] = {
            let axes = &mul112_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant1267_out1: [i64; 1] = [1i64];
        let gather99_out1: [i64; 1usize] = constant1267_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape119_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape119_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant1269_out1: [i64; 1] = [1i64];
        let add69_out1 = {
            let __lhs = gather99_out1;
            let __rhs = constant1269_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant1270_out1: [i64; 1] = [2i64];
        let div97_out1 = {
            let __lhs = add69_out1;
            let __rhs = constant1270_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| if __rhs[__i] != 0 { __lhs[__i] / __rhs[__i] } else { __lhs[__i] })
        };
        let slice117_out1 = mul112_out1.clone().slice(s![.., 0..div97_out1[0], .., ..]);
        let constant1272_out1: [i64; 1] = [2i64];
        let mul114_out1 = {
            let __lhs = div97_out1;
            let __rhs = constant1272_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice118_out1 = mul112_out1
            .slice(s![.., div97_out1[0]..mul114_out1[0], .., ..]);
        let conv2d12_out1 = self.conv2d12.forward(slice118_out1);
        let sigmoid3_out1 = burn::tensor::activation::sigmoid(conv2d12_out1.clone());
        let mul115_out1 = conv2d12_out1.mul(sigmoid3_out1);
        let conv2d13_out1 = self.conv2d13.forward(mul115_out1);
        let sigmoid4_out1 = burn::tensor::activation::sigmoid(conv2d13_out1.clone());
        let mul116_out1 = conv2d13_out1.mul(sigmoid4_out1);
        let add70_out1 = slice117_out1.add(mul116_out1);
        let conv2d14_out1 = self.conv2d14.forward(add70_out1.clone());
        let sigmoid5_out1 = burn::tensor::activation::sigmoid(conv2d14_out1.clone());
        let mul117_out1 = conv2d14_out1.mul(sigmoid5_out1);
        let shape120_out1: [i64; 4] = {
            let axes = &mul117_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant1273_out1: [i64; 1] = [1i64];
        let gather100_out1: [i64; 1usize] = constant1273_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape120_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape120_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant1275_out1: [i64; 1] = [1i64];
        let add71_out1 = {
            let __lhs = gather100_out1;
            let __rhs = constant1275_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant1276_out1: [i64; 1] = [2i64];
        let div98_out1 = {
            let __lhs = add71_out1;
            let __rhs = constant1276_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| if __rhs[__i] != 0 { __lhs[__i] / __rhs[__i] } else { __lhs[__i] })
        };
        let slice119_out1 = mul117_out1.clone().slice(s![.., 0..div98_out1[0], .., ..]);
        let constant1278_out1: [i64; 1] = [2i64];
        let mul119_out1 = {
            let __lhs = div98_out1;
            let __rhs = constant1278_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice120_out1 = mul117_out1
            .slice(s![.., div98_out1[0]..mul119_out1[0], .., ..]);
        let conv2d15_out1 = self.conv2d15.forward(slice120_out1);
        let sigmoid6_out1 = burn::tensor::activation::sigmoid(conv2d15_out1.clone());
        let mul120_out1 = conv2d15_out1.mul(sigmoid6_out1);
        let conv2d16_out1 = self.conv2d16.forward(mul120_out1);
        let sigmoid7_out1 = burn::tensor::activation::sigmoid(conv2d16_out1.clone());
        let mul121_out1 = conv2d16_out1.mul(sigmoid7_out1);
        let add72_out1 = slice119_out1.add(mul121_out1);
        let concat96_out1 = burn::tensor::Tensor::cat(
            [split13_out1, split13_out2, add70_out1, add72_out1].into(),
            1,
        );
        let conv2d17_out1 = self.conv2d17.forward(concat96_out1);
        let sigmoid8_out1 = burn::tensor::activation::sigmoid(conv2d17_out1.clone());
        let mul122_out1 = conv2d17_out1.mul(sigmoid8_out1);
        let conv2d18_out1 = self.conv2d18.forward(mul122_out1);
        let resize5_out1 = self.resize5.forward(conv2d18_out1.clone());
        let add73_out1 = resize5_out1.add(conv2d6_out1);
        let conv2d19_out1 = self.conv2d19.forward(add73_out1);
        let sigmoid9_out1 = burn::tensor::activation::sigmoid(conv2d19_out1.clone());
        let mul123_out1 = conv2d19_out1.mul(sigmoid9_out1);
        let [split14_out1, split14_out2] = {
            let split_tensors = mul123_out1.split_with_sizes([192, 192].into(), 1);
            split_tensors.try_into().unwrap()
        };
        let conv2d20_out1 = self.conv2d20.forward(split14_out2.clone());
        let sigmoid10_out1 = burn::tensor::activation::sigmoid(conv2d20_out1.clone());
        let mul124_out1 = conv2d20_out1.mul(sigmoid10_out1);
        let shape121_out1: [i64; 4] = {
            let axes = &mul124_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant1280_out1: [i64; 1] = [1i64];
        let gather101_out1: [i64; 1usize] = constant1280_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape121_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape121_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant1282_out1: [i64; 1] = [1i64];
        let add74_out1 = {
            let __lhs = gather101_out1;
            let __rhs = constant1282_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant1283_out1: [i64; 1] = [2i64];
        let div99_out1 = {
            let __lhs = add74_out1;
            let __rhs = constant1283_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| if __rhs[__i] != 0 { __lhs[__i] / __rhs[__i] } else { __lhs[__i] })
        };
        let slice121_out1 = mul124_out1.clone().slice(s![.., 0..div99_out1[0], .., ..]);
        let constant1285_out1: [i64; 1] = [2i64];
        let mul126_out1 = {
            let __lhs = div99_out1;
            let __rhs = constant1285_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice122_out1 = mul124_out1
            .slice(s![.., div99_out1[0]..mul126_out1[0], .., ..]);
        let conv2d21_out1 = self.conv2d21.forward(slice122_out1);
        let sigmoid11_out1 = burn::tensor::activation::sigmoid(conv2d21_out1.clone());
        let mul127_out1 = conv2d21_out1.mul(sigmoid11_out1);
        let conv2d22_out1 = self.conv2d22.forward(mul127_out1);
        let sigmoid12_out1 = burn::tensor::activation::sigmoid(conv2d22_out1.clone());
        let mul128_out1 = conv2d22_out1.mul(sigmoid12_out1);
        let add75_out1 = slice121_out1.add(mul128_out1);
        let conv2d23_out1 = self.conv2d23.forward(add75_out1.clone());
        let sigmoid13_out1 = burn::tensor::activation::sigmoid(conv2d23_out1.clone());
        let mul129_out1 = conv2d23_out1.mul(sigmoid13_out1);
        let shape122_out1: [i64; 4] = {
            let axes = &mul129_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant1286_out1: [i64; 1] = [1i64];
        let gather102_out1: [i64; 1usize] = constant1286_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape122_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape122_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant1288_out1: [i64; 1] = [1i64];
        let add76_out1 = {
            let __lhs = gather102_out1;
            let __rhs = constant1288_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant1289_out1: [i64; 1] = [2i64];
        let div100_out1 = {
            let __lhs = add76_out1;
            let __rhs = constant1289_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| if __rhs[__i] != 0 { __lhs[__i] / __rhs[__i] } else { __lhs[__i] })
        };
        let slice123_out1 = mul129_out1.clone().slice(s![.., 0..div100_out1[0], .., ..]);
        let constant1291_out1: [i64; 1] = [2i64];
        let mul131_out1 = {
            let __lhs = div100_out1;
            let __rhs = constant1291_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice124_out1 = mul129_out1
            .slice(s![.., div100_out1[0]..mul131_out1[0], .., ..]);
        let conv2d24_out1 = self.conv2d24.forward(slice124_out1);
        let sigmoid14_out1 = burn::tensor::activation::sigmoid(conv2d24_out1.clone());
        let mul132_out1 = conv2d24_out1.mul(sigmoid14_out1);
        let conv2d25_out1 = self.conv2d25.forward(mul132_out1);
        let sigmoid15_out1 = burn::tensor::activation::sigmoid(conv2d25_out1.clone());
        let mul133_out1 = conv2d25_out1.mul(sigmoid15_out1);
        let add77_out1 = slice123_out1.add(mul133_out1);
        let concat97_out1 = burn::tensor::Tensor::cat(
            [split14_out1, split14_out2, add75_out1, add77_out1].into(),
            1,
        );
        (concat97_out1, conv2d18_out1, conv2d9_out1)
    }
}
#[derive(Module, Debug)]
pub struct Submodule15 {
    conv2d26: Conv2d,
    conv2d27: Conv2d,
    conv2d28: Conv2d,
    conv2d29: Conv2d,
    conv2d30: Conv2d,
    conv2d31: Conv2d,
    conv2d32: Conv2d,
    conv2d33: Conv2d,
    conv2d34: Conv2d,
    conv2d35: Conv2d,
    conv2d36: Conv2d,
    conv2d37: Conv2d,
    conv2d38: Conv2d,
    conv2d39: Conv2d,
    conv2d40: Conv2d,
    conv2d41: Conv2d,
    conv2d42: Conv2d,
    conv2d43: Conv2d,
    conv2d44: Conv2d,
    conv2d45: Conv2d,
    device: AllocationDevice,
}
impl Submodule15 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let conv2d26 = Conv2dConfig::new([448, 192], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d27 = Conv2dConfig::new([192, 192], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d28 = Conv2dConfig::new([192, 192], [3, 3])
            .with_stride([2, 2])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(192)
            .with_bias(true)
            .init(device);
        let conv2d29 = Conv2dConfig::new([192, 384], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d30 = Conv2dConfig::new([192, 64], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d31 = Conv2dConfig::new([32, 32], [3, 3])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d32 = Conv2dConfig::new([32, 32], [3, 3])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d33 = Conv2dConfig::new([32, 64], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d34 = Conv2dConfig::new([32, 32], [3, 3])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d35 = Conv2dConfig::new([32, 32], [3, 3])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d36 = Conv2dConfig::new([448, 192], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d37 = Conv2dConfig::new([192, 192], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d38 = Conv2dConfig::new([192, 192], [3, 3])
            .with_stride([2, 2])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(192)
            .with_bias(true)
            .init(device);
        let conv2d39 = Conv2dConfig::new([192, 384], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d40 = Conv2dConfig::new([192, 64], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d41 = Conv2dConfig::new([32, 32], [3, 3])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d42 = Conv2dConfig::new([32, 32], [3, 3])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d43 = Conv2dConfig::new([32, 64], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d44 = Conv2dConfig::new([32, 32], [3, 3])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let conv2d45 = Conv2dConfig::new([32, 32], [3, 3])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        Self {
            conv2d26,
            conv2d27,
            conv2d28,
            conv2d29,
            conv2d30,
            conv2d31,
            conv2d32,
            conv2d33,
            conv2d34,
            conv2d35,
            conv2d36,
            conv2d37,
            conv2d38,
            conv2d39,
            conv2d40,
            conv2d41,
            conv2d42,
            conv2d43,
            conv2d44,
            conv2d45,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `concat97_out1`: expected dtype `DType::F32`
    /// - `conv2d18_out1`: expected dtype `DType::F32`
    /// - `conv2d9_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        concat97_out1: Tensor<4>,
        conv2d18_out1: Tensor<4>,
        conv2d9_out1: Tensor<4>,
    ) -> (Tensor<4>, Tensor<4>, Tensor<4>) {
        let conv2d26_out1 = self.conv2d26.forward(concat97_out1);
        let sigmoid16_out1 = burn::tensor::activation::sigmoid(conv2d26_out1.clone());
        let mul134_out1 = conv2d26_out1.mul(sigmoid16_out1);
        let conv2d27_out1 = self.conv2d27.forward(mul134_out1.clone());
        let conv2d28_out1 = self.conv2d28.forward(conv2d27_out1);
        let add78_out1 = conv2d28_out1.add(conv2d18_out1);
        let conv2d29_out1 = self.conv2d29.forward(add78_out1);
        let sigmoid17_out1 = burn::tensor::activation::sigmoid(conv2d29_out1.clone());
        let mul135_out1 = conv2d29_out1.mul(sigmoid17_out1);
        let [split15_out1, split15_out2] = {
            let split_tensors = mul135_out1.split_with_sizes([192, 192].into(), 1);
            split_tensors.try_into().unwrap()
        };
        let conv2d30_out1 = self.conv2d30.forward(split15_out2.clone());
        let sigmoid18_out1 = burn::tensor::activation::sigmoid(conv2d30_out1.clone());
        let mul136_out1 = conv2d30_out1.mul(sigmoid18_out1);
        let shape123_out1: [i64; 4] = {
            let axes = &mul136_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant1292_out1: [i64; 1] = [1i64];
        let gather103_out1: [i64; 1usize] = constant1292_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape123_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape123_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant1294_out1: [i64; 1] = [1i64];
        let add79_out1 = {
            let __lhs = gather103_out1;
            let __rhs = constant1294_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant1295_out1: [i64; 1] = [2i64];
        let div101_out1 = {
            let __lhs = add79_out1;
            let __rhs = constant1295_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| if __rhs[__i] != 0 { __lhs[__i] / __rhs[__i] } else { __lhs[__i] })
        };
        let slice125_out1 = mul136_out1.clone().slice(s![.., 0..div101_out1[0], .., ..]);
        let constant1297_out1: [i64; 1] = [2i64];
        let mul138_out1 = {
            let __lhs = div101_out1;
            let __rhs = constant1297_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice126_out1 = mul136_out1
            .slice(s![.., div101_out1[0]..mul138_out1[0], .., ..]);
        let conv2d31_out1 = self.conv2d31.forward(slice126_out1);
        let sigmoid19_out1 = burn::tensor::activation::sigmoid(conv2d31_out1.clone());
        let mul139_out1 = conv2d31_out1.mul(sigmoid19_out1);
        let conv2d32_out1 = self.conv2d32.forward(mul139_out1);
        let sigmoid20_out1 = burn::tensor::activation::sigmoid(conv2d32_out1.clone());
        let mul140_out1 = conv2d32_out1.mul(sigmoid20_out1);
        let add80_out1 = slice125_out1.add(mul140_out1);
        let conv2d33_out1 = self.conv2d33.forward(add80_out1.clone());
        let sigmoid21_out1 = burn::tensor::activation::sigmoid(conv2d33_out1.clone());
        let mul141_out1 = conv2d33_out1.mul(sigmoid21_out1);
        let shape124_out1: [i64; 4] = {
            let axes = &mul141_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant1298_out1: [i64; 1] = [1i64];
        let gather104_out1: [i64; 1usize] = constant1298_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape124_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape124_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant1300_out1: [i64; 1] = [1i64];
        let add81_out1 = {
            let __lhs = gather104_out1;
            let __rhs = constant1300_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant1301_out1: [i64; 1] = [2i64];
        let div102_out1 = {
            let __lhs = add81_out1;
            let __rhs = constant1301_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| if __rhs[__i] != 0 { __lhs[__i] / __rhs[__i] } else { __lhs[__i] })
        };
        let slice127_out1 = mul141_out1.clone().slice(s![.., 0..div102_out1[0], .., ..]);
        let constant1303_out1: [i64; 1] = [2i64];
        let mul143_out1 = {
            let __lhs = div102_out1;
            let __rhs = constant1303_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice128_out1 = mul141_out1
            .slice(s![.., div102_out1[0]..mul143_out1[0], .., ..]);
        let conv2d34_out1 = self.conv2d34.forward(slice128_out1);
        let sigmoid22_out1 = burn::tensor::activation::sigmoid(conv2d34_out1.clone());
        let mul144_out1 = conv2d34_out1.mul(sigmoid22_out1);
        let conv2d35_out1 = self.conv2d35.forward(mul144_out1);
        let sigmoid23_out1 = burn::tensor::activation::sigmoid(conv2d35_out1.clone());
        let mul145_out1 = conv2d35_out1.mul(sigmoid23_out1);
        let add82_out1 = slice127_out1.add(mul145_out1);
        let concat98_out1 = burn::tensor::Tensor::cat(
            [split15_out1, split15_out2, add80_out1, add82_out1].into(),
            1,
        );
        let conv2d36_out1 = self.conv2d36.forward(concat98_out1);
        let sigmoid24_out1 = burn::tensor::activation::sigmoid(conv2d36_out1.clone());
        let mul146_out1 = conv2d36_out1.mul(sigmoid24_out1);
        let conv2d37_out1 = self.conv2d37.forward(mul146_out1.clone());
        let conv2d38_out1 = self.conv2d38.forward(conv2d37_out1);
        let add83_out1 = conv2d38_out1.add(conv2d9_out1);
        let conv2d39_out1 = self.conv2d39.forward(add83_out1);
        let sigmoid25_out1 = burn::tensor::activation::sigmoid(conv2d39_out1.clone());
        let mul147_out1 = conv2d39_out1.mul(sigmoid25_out1);
        let [split16_out1, split16_out2] = {
            let split_tensors = mul147_out1.split_with_sizes([192, 192].into(), 1);
            split_tensors.try_into().unwrap()
        };
        let conv2d40_out1 = self.conv2d40.forward(split16_out2.clone());
        let sigmoid26_out1 = burn::tensor::activation::sigmoid(conv2d40_out1.clone());
        let mul148_out1 = conv2d40_out1.mul(sigmoid26_out1);
        let shape125_out1: [i64; 4] = {
            let axes = &mul148_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant1304_out1: [i64; 1] = [1i64];
        let gather105_out1: [i64; 1usize] = constant1304_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape125_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape125_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant1306_out1: [i64; 1] = [1i64];
        let add84_out1 = {
            let __lhs = gather105_out1;
            let __rhs = constant1306_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant1307_out1: [i64; 1] = [2i64];
        let div103_out1 = {
            let __lhs = add84_out1;
            let __rhs = constant1307_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| if __rhs[__i] != 0 { __lhs[__i] / __rhs[__i] } else { __lhs[__i] })
        };
        let slice129_out1 = mul148_out1.clone().slice(s![.., 0..div103_out1[0], .., ..]);
        let constant1309_out1: [i64; 1] = [2i64];
        let mul150_out1 = {
            let __lhs = div103_out1;
            let __rhs = constant1309_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice130_out1 = mul148_out1
            .slice(s![.., div103_out1[0]..mul150_out1[0], .., ..]);
        let conv2d41_out1 = self.conv2d41.forward(slice130_out1);
        let sigmoid27_out1 = burn::tensor::activation::sigmoid(conv2d41_out1.clone());
        let mul151_out1 = conv2d41_out1.mul(sigmoid27_out1);
        let conv2d42_out1 = self.conv2d42.forward(mul151_out1);
        let sigmoid28_out1 = burn::tensor::activation::sigmoid(conv2d42_out1.clone());
        let mul152_out1 = conv2d42_out1.mul(sigmoid28_out1);
        let add85_out1 = slice129_out1.add(mul152_out1);
        let conv2d43_out1 = self.conv2d43.forward(add85_out1.clone());
        let sigmoid29_out1 = burn::tensor::activation::sigmoid(conv2d43_out1.clone());
        let mul153_out1 = conv2d43_out1.mul(sigmoid29_out1);
        let shape126_out1: [i64; 4] = {
            let axes = &mul153_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant1310_out1: [i64; 1] = [1i64];
        let gather106_out1: [i64; 1usize] = constant1310_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape126_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape126_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant1312_out1: [i64; 1] = [1i64];
        let add86_out1 = {
            let __lhs = gather106_out1;
            let __rhs = constant1312_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant1313_out1: [i64; 1] = [2i64];
        let div104_out1 = {
            let __lhs = add86_out1;
            let __rhs = constant1313_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| if __rhs[__i] != 0 { __lhs[__i] / __rhs[__i] } else { __lhs[__i] })
        };
        let slice131_out1 = mul153_out1.clone().slice(s![.., 0..div104_out1[0], .., ..]);
        let constant1315_out1: [i64; 1] = [2i64];
        let mul155_out1 = {
            let __lhs = div104_out1;
            let __rhs = constant1315_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice132_out1 = mul153_out1
            .slice(s![.., div104_out1[0]..mul155_out1[0], .., ..]);
        let conv2d44_out1 = self.conv2d44.forward(slice132_out1);
        let sigmoid30_out1 = burn::tensor::activation::sigmoid(conv2d44_out1.clone());
        let mul156_out1 = conv2d44_out1.mul(sigmoid30_out1);
        let conv2d45_out1 = self.conv2d45.forward(mul156_out1);
        let sigmoid31_out1 = burn::tensor::activation::sigmoid(conv2d45_out1.clone());
        let mul157_out1 = conv2d45_out1.mul(sigmoid31_out1);
        let add87_out1 = slice131_out1.add(mul157_out1);
        let concat99_out1 = burn::tensor::Tensor::cat(
            [split16_out1, split16_out2, add85_out1, add87_out1].into(),
            1,
        );
        (concat99_out1, mul134_out1, mul146_out1)
    }
}
#[derive(Module, Debug)]
pub struct Submodule16 {
    conv2d46: Conv2d,
    constant323: burn::module::Param<Tensor<3>>,
    linear55: Linear,
    constant101: burn::module::Param<Tensor<3>>,
    linear56: Linear,
    linear57: Linear,
    linear58: Linear,
    device: AllocationDevice,
}
impl Submodule16 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let conv2d46 = Conv2dConfig::new([448, 192], [1, 1])
            .with_stride([1, 1])
            .with_padding(PaddingConfig2d::Valid)
            .with_dilation([1, 1])
            .with_groups(1)
            .with_bias(true)
            .init(device);
        let constant323: burn::module::Param<Tensor<3>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                3,
            >::zeros([1, 13125, 1], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1, 13125, 1].into(),
        );
        let linear55 = LinearConfig::new(192, 18).with_bias(true).init(device);
        let constant101: burn::module::Param<Tensor<3>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                3,
            >::zeros([1, 13125, 4], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1, 13125, 4].into(),
        );
        let linear56 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear57 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear58 = LinearConfig::new(192, 4).with_bias(true).init(device);
        Self {
            conv2d46,
            constant323,
            linear55,
            constant101,
            linear56,
            linear57,
            linear58,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `concat99_out1`: expected dtype `DType::F32`
    /// - `mul134_out1`: expected dtype `DType::F32`
    /// - `mul146_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        concat99_out1: Tensor<4>,
        mul134_out1: Tensor<4>,
        mul146_out1: Tensor<4>,
    ) -> (i64, i64, i64, i64, i64, i64, Tensor<4>, Tensor<3>, Tensor<3>) {
        let conv2d46_out1 = self.conv2d46.forward(concat99_out1);
        let sigmoid32_out1 = burn::tensor::activation::sigmoid(conv2d46_out1.clone());
        let mul158_out1 = conv2d46_out1.mul(sigmoid32_out1);
        let shape127_out1: [i64; 4] = {
            let axes = &mul134_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather107_out1 = shape127_out1[2] as i64;
        let gather108_out1 = shape127_out1[3] as i64;
        let slice133_out1: [i64; 2] = shape127_out1[0..2].try_into().unwrap();
        let constant1321_out1: [i64; 1] = [-1i64];
        let concat100_out1: [i64; 3usize] = [&slice133_out1[..], &constant1321_out1[..]]
            .concat()
            .try_into()
            .unwrap();
        let reshape41_out1 = mul134_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat100_out1));
        let transpose50_out1 = reshape41_out1.permute([0, 2, 1]);
        let shape130_out1: [i64; 4] = {
            let axes = &mul146_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather109_out1 = shape130_out1[2] as i64;
        let gather110_out1 = shape130_out1[3] as i64;
        let slice134_out1: [i64; 2] = shape130_out1[0..2].try_into().unwrap();
        let constant1327_out1: [i64; 1] = [-1i64];
        let concat101_out1: [i64; 3usize] = [&slice134_out1[..], &constant1327_out1[..]]
            .concat()
            .try_into()
            .unwrap();
        let reshape42_out1 = mul146_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat101_out1));
        let transpose51_out1 = reshape42_out1.permute([0, 2, 1]);
        let shape133_out1: [i64; 4] = {
            let axes = &mul158_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather111_out1 = shape133_out1[2] as i64;
        let gather112_out1 = shape133_out1[3] as i64;
        let slice135_out1: [i64; 2] = shape133_out1[0..2].try_into().unwrap();
        let constant1333_out1: [i64; 1] = [-1i64];
        let concat102_out1: [i64; 3usize] = [&slice135_out1[..], &constant1333_out1[..]]
            .concat()
            .try_into()
            .unwrap();
        let reshape43_out1 = mul158_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat102_out1));
        let transpose52_out1 = reshape43_out1.permute([0, 2, 1]);
        let concat103_out1 = burn::tensor::Tensor::cat(
            [transpose50_out1, transpose51_out1, transpose52_out1].into(),
            1,
        );
        let constant323_out1 = self.constant323.val();
        let mul159_out1 = constant323_out1.mul(concat103_out1.clone());
        let linear55_out1 = self.linear55.forward(mul159_out1.clone());
        let reducemax1_out1 = {
            linear55_out1.max_dim(2usize).squeeze_dims::<2usize>(&[2])
        };
        let (topk1_out1, topk1_out2) = {
            let (values, indices) = reducemax1_out1.topk_with_indices(1200, 1);
            (values, indices.cast(burn::tensor::DType::I64))
        };
        let unsqueeze154_out1: Tensor<3, Int> = topk1_out2.unsqueeze_dims::<3>(&[-1]);
        let expand5_out1 = {
            let onnx_shape: [i64; 3usize] = [1, 1, 1];
            let input_dims = unsqueeze154_out1.clone().dims();
            let mut shape = onnx_shape;
            #[allow(clippy::needless_range_loop)]
            for i in 0..3usize {
                let dim_offset = i;
                if shape[dim_offset] == 1 && input_dims[i] > 1 {
                    shape[dim_offset] = input_dims[i] as i64;
                }
            }
            unsqueeze154_out1.clone().expand(shape)
        };
        let tile2_out1 = expand5_out1.repeat(&[1, 1, 4]);
        let constant101_out1 = self.constant101.val();
        let gatherelements1_out1 = {
            let axis_size = constant101_out1.dims()[1] as i64;
            let negative = tile2_out1.clone().lower_elem(0i64);
            let corrected = tile2_out1.clone() + axis_size;
            let indices = tile2_out1.mask_where(negative, corrected);
            constant101_out1.gather(1, indices)
        };
        let shape136_out1: [i64; 3] = {
            let axes = &mul159_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather113_out1 = shape136_out1[2] as i64;
        let unsqueeze156_out1 = [gather113_out1 as i64];
        let constant1342_out1: [i64; 1] = [1i64];
        let constant1343_out1: [i64; 1] = [1i64];
        let concat105_out1: [i64; 3usize] = [
            &constant1342_out1[..],
            &constant1343_out1[..],
            &unsqueeze156_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let expand6_out1 = {
            let onnx_shape: [i64; 3usize] = [1, 1, 1];
            let input_dims = unsqueeze154_out1.dims();
            let mut shape = onnx_shape;
            #[allow(clippy::needless_range_loop)]
            for i in 0..3usize {
                let dim_offset = i;
                if shape[dim_offset] == 1 && input_dims[i] > 1 {
                    shape[dim_offset] = input_dims[i] as i64;
                }
            }
            unsqueeze154_out1.expand(shape)
        };
        let tile3_out1 = {
            let repeats: alloc::vec::Vec<usize> = concat105_out1
                .iter()
                .map(|&v| v as usize)
                .collect();
            expand6_out1.repeat(&repeats)
        };
        let gatherelements2_out1 = {
            let axis_size = mul159_out1.dims()[1] as i64;
            let negative = tile3_out1.clone().lower_elem(0i64);
            let corrected = tile3_out1.clone() + axis_size;
            let indices = tile3_out1.mask_where(negative, corrected);
            mul159_out1.gather(1, indices)
        };
        let linear56_out1 = self.linear56.forward(gatherelements2_out1.clone());
        let sigmoid33_out1 = burn::tensor::activation::sigmoid(linear56_out1.clone());
        let mul160_out1 = linear56_out1.mul(sigmoid33_out1);
        let linear57_out1 = self.linear57.forward(mul160_out1);
        let sigmoid34_out1 = burn::tensor::activation::sigmoid(linear57_out1.clone());
        let mul161_out1 = linear57_out1.mul(sigmoid34_out1);
        let linear58_out1 = self.linear58.forward(mul161_out1);
        let add88_out1 = linear58_out1.add(gatherelements1_out1);
        let shape138_out1: [i64; 3] = {
            let axes = &concat103_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather114_out1 = shape138_out1[0] as i64;
        let gather115_out1 = shape138_out1[1] as i64;
        let unsqueeze157_out1 = [gather114_out1 as i64];
        let unsqueeze158_out1 = [gather115_out1 as i64];
        let constant1350_out1: [i64; 1] = [-1i64];
        let constant1349_out1: [i64; 1] = [8i64];
        let concat106_out1: [i64; 4usize] = [
            &unsqueeze157_out1[..],
            &unsqueeze158_out1[..],
            &constant1349_out1[..],
            &constant1350_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape44_out1 = concat103_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat106_out1));
        (
            gather107_out1,
            gather108_out1,
            gather109_out1,
            gather110_out1,
            gather111_out1,
            gather112_out1,
            reshape44_out1,
            add88_out1,
            gatherelements2_out1,
        )
    }
}
#[derive(Module, Debug)]
pub struct Submodule17 {
    linear59: Linear,
    linear60: Linear,
    linear61: Linear,
    linear62: Linear,
    linear63: Linear,
    linear64: Linear,
    constant1376: burn::module::Param<Tensor<1>>,
    linear65: Linear,
    device: AllocationDevice,
}
impl Submodule17 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let linear59 = LinearConfig::new(4, 192).with_bias(true).init(device);
        let linear60 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear61 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear62 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear63 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear64 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let constant1376: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.20412415266036987f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear65 = LinearConfig::new(192, 192).with_bias(true).init(device);
        Self {
            linear59,
            linear60,
            linear61,
            linear62,
            linear63,
            linear64,
            constant1376,
            linear65,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `reshape44_out1`: expected dtype `DType::F32`
    /// - `add88_out1`: expected dtype `DType::F32`
    /// - `gatherelements2_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        gather107_out1: i64,
        gather108_out1: i64,
        gather109_out1: i64,
        gather110_out1: i64,
        gather111_out1: i64,
        gather112_out1: i64,
        reshape44_out1: Tensor<4>,
        add88_out1: Tensor<3>,
        gatherelements2_out1: Tensor<3>,
    ) -> (Tensor<3>, Tensor<3>, Tensor<4>, Tensor<4>, Tensor<4>, Tensor<4>, Tensor<3>) {
        let mul162_out1 = gather107_out1 * gather108_out1;
        let mul163_out1 = gather109_out1 * gather110_out1;
        let mul164_out1 = gather111_out1 * gather112_out1;
        let transpose53_out1 = reshape44_out1.permute([0, 2, 3, 1]);
        let unsqueeze159_out1 = [mul162_out1 as i64];
        let unsqueeze160_out1 = [mul163_out1 as i64];
        let unsqueeze161_out1 = [mul164_out1 as i64];
        let slice136_out1 = transpose53_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze159_out1[0]]);
        let add90_out1 = {
            let __lhs = unsqueeze159_out1;
            let __rhs = unsqueeze160_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let slice137_out1 = transpose53_out1
            .clone()
            .slice(s![.., .., .., unsqueeze159_out1[0]..add90_out1[0]]);
        let add91_out1 = {
            let __lhs = add90_out1;
            let __rhs = unsqueeze161_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let slice138_out1 = transpose53_out1
            .slice(s![.., .., .., add90_out1[0]..add91_out1[0]]);
        let sigmoid35_out1 = burn::tensor::activation::sigmoid(add88_out1);
        let linear59_out1 = self.linear59.forward(sigmoid35_out1.clone());
        let sigmoid36_out1 = burn::tensor::activation::sigmoid(linear59_out1.clone());
        let mul165_out1 = linear59_out1.mul(sigmoid36_out1);
        let linear60_out1 = self.linear60.forward(mul165_out1);
        let sigmoid37_out1 = burn::tensor::activation::sigmoid(linear60_out1.clone());
        let mul166_out1 = linear60_out1.mul(sigmoid37_out1);
        let linear61_out1 = self.linear61.forward(mul166_out1);
        let clip1_out1 = linear61_out1.clamp(-10f64, 10f64);
        let unsqueeze162_out1: Tensor<4> = sigmoid35_out1
            .clone()
            .unsqueeze_dims::<4>(&[2]);
        let add92_out1 = gatherelements2_out1.clone().add(clip1_out1.clone());
        let transpose54_out1 = add92_out1.permute([1, 0, 2]);
        let transpose55_out1 = gatherelements2_out1.clone().permute([1, 0, 2]);
        let shape140_out1: [i64; 3] = {
            let axes = &transpose54_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather116_out1 = shape140_out1[0] as i64;
        let gather117_out1 = shape140_out1[1] as i64;
        let gather118_out1 = shape140_out1[2] as i64;
        let constant1363_out1 = 8i64;
        let div105_out1 = gather118_out1 / constant1363_out1;
        let linear62_out1 = self.linear62.forward(transpose54_out1.clone());
        let linear63_out1 = self.linear63.forward(transpose54_out1);
        let linear64_out1 = self.linear64.forward(transpose55_out1);
        let constant1364_out1 = 8i64;
        let mul167_out1 = gather117_out1 * constant1364_out1;
        let unsqueeze163_out1 = [gather116_out1 as i64];
        let unsqueeze164_out1 = [mul167_out1 as i64];
        let unsqueeze165_out1 = [div105_out1 as i64];
        let concat107_out1: [i64; 3usize] = [
            &unsqueeze163_out1[..],
            &unsqueeze164_out1[..],
            &unsqueeze165_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape45_out1 = linear62_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat107_out1));
        let transpose56_out1 = reshape45_out1.permute([1, 0, 2]);
        let shape143_out1: [i64; 3] = {
            let axes = &linear63_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather119_out1 = shape143_out1[0] as i64;
        let unsqueeze166_out1 = [gather119_out1 as i64];
        let unsqueeze167_out1 = [mul167_out1 as i64];
        let unsqueeze168_out1 = [div105_out1 as i64];
        let concat108_out1: [i64; 3usize] = [
            &unsqueeze166_out1[..],
            &unsqueeze167_out1[..],
            &unsqueeze168_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape46_out1 = linear63_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat108_out1));
        let shape144_out1: [i64; 3] = {
            let axes = &linear64_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather120_out1 = shape144_out1[0] as i64;
        let unsqueeze169_out1 = [gather120_out1 as i64];
        let unsqueeze170_out1 = [mul167_out1 as i64];
        let unsqueeze171_out1 = [div105_out1 as i64];
        let concat109_out1: [i64; 3usize] = [
            &unsqueeze169_out1[..],
            &unsqueeze170_out1[..],
            &unsqueeze171_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape47_out1 = linear64_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat109_out1));
        let transpose57_out1 = reshape47_out1.permute([1, 0, 2]);
        let constant1376_out1 = self.constant1376.val();
        let mul168_out1 = transpose56_out1
            .mul((constant1376_out1).unsqueeze_dims(&[0isize, 1isize]));
        let transpose58_out1 = reshape46_out1.permute([1, 2, 0]);
        let matmul90_out1 = mul168_out1.matmul(transpose58_out1);
        let softmax14_out1 = burn::tensor::activation::softmax(matmul90_out1, 2);
        let matmul91_out1 = softmax14_out1.matmul(transpose57_out1);
        let transpose59_out1 = matmul91_out1.permute([1, 0, 2]);
        let mul169_out1 = gather116_out1 * gather117_out1;
        let unsqueeze172_out1 = [mul169_out1 as i64];
        let unsqueeze173_out1 = [gather118_out1 as i64];
        let concat110_out1: [i64; 2usize] = [
            &unsqueeze172_out1[..],
            &unsqueeze173_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape48_out1 = transpose59_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat110_out1));
        let linear65_out1 = self.linear65.forward(reshape48_out1);
        let shape145_out1: [i64; 2] = {
            let axes = &linear65_out1.clone().dims()[0..2];
            let mut output = [0i64; 2];
            for i in 0..2 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather121_out1 = shape145_out1[1] as i64;
        let unsqueeze174_out1 = [gather116_out1 as i64];
        let unsqueeze175_out1 = [gather117_out1 as i64];
        let unsqueeze176_out1 = [gather121_out1 as i64];
        let concat111_out1: [i64; 3usize] = [
            &unsqueeze174_out1[..],
            &unsqueeze175_out1[..],
            &unsqueeze176_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape49_out1 = linear65_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat111_out1));
        let transpose60_out1 = reshape49_out1.permute([1, 0, 2]);
        let add93_out1 = gatherelements2_out1.add(transpose60_out1);
        (
            add93_out1,
            clip1_out1,
            unsqueeze162_out1,
            slice136_out1,
            slice137_out1,
            slice138_out1,
            sigmoid35_out1,
        )
    }
}
#[derive(Module, Debug)]
pub struct Submodule18 {
    constant1383: burn::module::Param<Tensor<1>>,
    constant1385: burn::module::Param<Tensor<1>>,
    constant1386: burn::module::Param<Tensor<1>>,
    constant104: burn::module::Param<Tensor<1>>,
    linear66: Linear,
    linear67: Linear,
    constant339: burn::module::Param<Tensor<2>>,
    constant1403: burn::module::Param<Tensor<1>>,
    constant1412: burn::module::Param<Tensor<1>>,
    constant1413: burn::module::Param<Tensor<1>>,
    linear68: Linear,
    device: AllocationDevice,
}
impl Submodule18 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let constant1383: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1385: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.0000009999999974752427f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1386: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant104: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([192], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [192].into(),
        );
        let linear66 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear67 = LinearConfig::new(192, 96).with_bias(true).init(device);
        let constant339: burn::module::Param<Tensor<2>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                2,
            >::zeros([12, 1], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [12, 1].into(),
        );
        let constant1403: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1412: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1413: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear68 = LinearConfig::new(384, 384).with_bias(true).init(device);
        Self {
            constant1383,
            constant1385,
            constant1386,
            constant104,
            linear66,
            linear67,
            constant339,
            constant1403,
            constant1412,
            constant1413,
            linear68,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `add93_out1`: expected dtype `DType::F32`
    /// - `clip1_out1`: expected dtype `DType::F32`
    /// - `unsqueeze162_out1`: expected dtype `DType::F32`
    /// - `slice136_out1`: expected dtype `DType::F32`
    /// - `slice137_out1`: expected dtype `DType::F32`
    /// - `slice138_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        add93_out1: Tensor<3>,
        clip1_out1: Tensor<3>,
        unsqueeze162_out1: Tensor<4>,
        slice136_out1: Tensor<4>,
        gather107_out1: i64,
        gather108_out1: i64,
        gather109_out1: i64,
        gather110_out1: i64,
        slice137_out1: Tensor<4>,
        gather111_out1: i64,
        gather112_out1: i64,
        slice138_out1: Tensor<4>,
    ) -> (Tensor<3>, Tensor<2>, [i64; 4]) {
        let constant1383_out1 = self.constant1383.val();
        let pow1_out1 = add93_out1
            .clone()
            .powf((constant1383_out1).unsqueeze_dims(&[0isize, 1isize]));
        let reducemean1_out1 = { pow1_out1.mean_dim(2usize) };
        let constant1385_out1 = self.constant1385.val();
        let add94_out1 = reducemean1_out1
            .add((constant1385_out1).unsqueeze_dims(&[0isize, 1isize]));
        let sqrt37_out1 = add94_out1.sqrt();
        let constant1386_out1 = self.constant1386.val();
        let div106_out1 = (constant1386_out1)
            .unsqueeze_dims(&[0isize, 1isize])
            .div(sqrt37_out1);
        let mul170_out1 = add93_out1.mul(div106_out1);
        let constant104_out1 = self.constant104.val();
        let mul171_out1 = mul170_out1
            .mul((constant104_out1).unsqueeze_dims(&[0isize, 1isize]));
        let add95_out1 = mul171_out1.clone().add(clip1_out1);
        let shape146_out1: [i64; 3] = {
            let axes = &add95_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather122_out1 = shape146_out1[0] as i64;
        let gather123_out1 = shape146_out1[1] as i64;
        let linear66_out1 = self.linear66.forward(add95_out1.clone());
        let unsqueeze177_out1 = [gather122_out1 as i64];
        let unsqueeze178_out1 = [gather123_out1 as i64];
        let constant1393_out1: [i64; 1] = [2i64];
        let constant1391_out1: [i64; 1] = [8i64];
        let constant1392_out1: [i64; 1] = [12i64];
        let concat112_out1: [i64; 5usize] = [
            &unsqueeze177_out1[..],
            &unsqueeze178_out1[..],
            &constant1391_out1[..],
            &constant1392_out1[..],
            &constant1393_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape50_out1 = linear66_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat112_out1));
        let linear67_out1 = self.linear67.forward(add95_out1);
        let unsqueeze179_out1 = [gather122_out1 as i64];
        let unsqueeze180_out1 = [gather123_out1 as i64];
        let constant1397_out1: [i64; 1] = [12i64];
        let constant1396_out1: [i64; 1] = [8i64];
        let concat113_out1: [i64; 4usize] = [
            &unsqueeze179_out1[..],
            &unsqueeze180_out1[..],
            &constant1396_out1[..],
            &constant1397_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape51_out1 = linear67_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat113_out1));
        let softmax15_out1 = burn::tensor::activation::softmax(reshape51_out1, 3);
        let constant339_out1 = self.constant339.val();
        let mul172_out1 = reshape50_out1
            .mul((constant339_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let unsqueeze181_out1: Tensor<5> = unsqueeze162_out1.unsqueeze_dims::<5>(&[2]);
        let slice139_out1 = unsqueeze181_out1.clone().slice(s![.., .., .., .., 2..]);
        let mul173_out1 = mul172_out1.mul(slice139_out1);
        let constant1403_out1 = self.constant1403.val();
        let mul174_out1 = mul173_out1
            .mul((constant1403_out1).unsqueeze_dims(&[0isize, 1isize, 2isize, 3isize]));
        let slice140_out1 = unsqueeze181_out1.slice(s![.., .., .., .., 0..2]);
        let add96_out1 = slice140_out1.add(mul174_out1);
        let shape148_out1: [i64; 4] = {
            let axes = &slice136_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather124_out1 = shape148_out1[0] as i64;
        let gather125_out1 = shape148_out1[1] as i64;
        let gather126_out1 = shape148_out1[2] as i64;
        let shape151_out1: [i64; 5] = {
            let axes = &add96_out1.clone().dims()[0..5];
            let mut output = [0i64; 5];
            for i in 0..5 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather127_out1 = shape151_out1[1] as i64;
        let constant1412_out1 = self.constant1412.val();
        let mul175_out1 = add96_out1
            .mul((constant1412_out1).unsqueeze_dims(&[0isize, 1isize, 2isize, 3isize]));
        let constant1413_out1 = self.constant1413.val();
        let sub2_out1 = mul175_out1
            .sub((constant1413_out1).unsqueeze_dims(&[0isize, 1isize, 2isize, 3isize]));
        let transpose61_out1 = sub2_out1.permute([0, 2, 1, 3, 4]);
        let shape152_out1: [i64; 5] = {
            let axes = &transpose61_out1.clone().dims()[0..5];
            let mut output = [0i64; 5];
            for i in 0..5 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice141_out1: [i64; 0] = shape152_out1[0..0].try_into().unwrap();
        let slice142_out1: [i64; 3] = shape152_out1[2..5].try_into().unwrap();
        let constant1420_out1: [i64; 1] = [-1i64];
        let concat114_out1: [i64; 4usize] = [
            &slice141_out1[..],
            &constant1420_out1[..],
            &slice142_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape52_out1 = transpose61_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat114_out1));
        let [split17_out1, split17_out2, split17_out3] = {
            let split_tensors = reshape52_out1.split_with_sizes([3, 6, 3].into(), 2);
            split_tensors.try_into().unwrap()
        };
        let mul176_out1 = gather124_out1 * gather125_out1;
        let unsqueeze182_out1 = [mul176_out1 as i64];
        let unsqueeze183_out1 = [gather126_out1 as i64];
        let unsqueeze184_out1 = [gather107_out1 as i64];
        let unsqueeze185_out1 = [gather108_out1 as i64];
        let concat115_out1: [i64; 4usize] = [
            &unsqueeze182_out1[..],
            &unsqueeze183_out1[..],
            &unsqueeze184_out1[..],
            &unsqueeze185_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape53_out1 = slice136_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat115_out1));
        let gridsample1_out1 = reshape53_out1
            .grid_sample_2d(
                split17_out1,
                burn::tensor::ops::GridSampleOptions::new(
                        burn::tensor::ops::GridSampleMode::Bilinear,
                    )
                    .with_padding_mode(burn::tensor::ops::GridSamplePaddingMode::Zeros)
                    .with_align_corners(false),
            );
        let unsqueeze186_out1 = [mul176_out1 as i64];
        let unsqueeze187_out1 = [gather126_out1 as i64];
        let unsqueeze188_out1 = [gather109_out1 as i64];
        let unsqueeze189_out1 = [gather110_out1 as i64];
        let concat116_out1: [i64; 4usize] = [
            &unsqueeze186_out1[..],
            &unsqueeze187_out1[..],
            &unsqueeze188_out1[..],
            &unsqueeze189_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape54_out1 = slice137_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat116_out1));
        let gridsample2_out1 = reshape54_out1
            .grid_sample_2d(
                split17_out2,
                burn::tensor::ops::GridSampleOptions::new(
                        burn::tensor::ops::GridSampleMode::Bilinear,
                    )
                    .with_padding_mode(burn::tensor::ops::GridSamplePaddingMode::Zeros)
                    .with_align_corners(false),
            );
        let unsqueeze190_out1 = [mul176_out1 as i64];
        let unsqueeze191_out1 = [gather126_out1 as i64];
        let unsqueeze192_out1 = [gather111_out1 as i64];
        let unsqueeze193_out1 = [gather112_out1 as i64];
        let concat117_out1: [i64; 4usize] = [
            &unsqueeze190_out1[..],
            &unsqueeze191_out1[..],
            &unsqueeze192_out1[..],
            &unsqueeze193_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape55_out1 = slice138_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat117_out1));
        let gridsample3_out1 = reshape55_out1
            .grid_sample_2d(
                split17_out3,
                burn::tensor::ops::GridSampleOptions::new(
                        burn::tensor::ops::GridSampleMode::Bilinear,
                    )
                    .with_padding_mode(burn::tensor::ops::GridSamplePaddingMode::Zeros)
                    .with_align_corners(false),
            );
        let transpose62_out1 = softmax15_out1.permute([0, 2, 1, 3]);
        let unsqueeze194_out1 = [mul176_out1 as i64];
        let unsqueeze195_out1 = [gather127_out1 as i64];
        let constant1437_out1: [i64; 1] = [12i64];
        let constant1435_out1: [i64; 1] = [1i64];
        let concat118_out1: [i64; 4usize] = [
            &unsqueeze194_out1[..],
            &constant1435_out1[..],
            &unsqueeze195_out1[..],
            &constant1437_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape56_out1 = transpose62_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat118_out1));
        let concat119_out1 = burn::tensor::Tensor::cat(
            [gridsample1_out1, gridsample2_out1, gridsample3_out1].into(),
            3,
        );
        let mul177_out1 = concat119_out1.mul(reshape56_out1);
        let reducesum1_out1 = {
            mul177_out1.sum_dim(3usize).squeeze_dims::<3usize>(&[3])
        };
        let mul178_out1 = gather125_out1 * gather126_out1;
        let unsqueeze196_out1 = [gather124_out1 as i64];
        let unsqueeze197_out1 = [mul178_out1 as i64];
        let unsqueeze198_out1 = [gather127_out1 as i64];
        let concat120_out1: [i64; 3usize] = [
            &unsqueeze196_out1[..],
            &unsqueeze197_out1[..],
            &unsqueeze198_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape57_out1 = reducesum1_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat120_out1));
        let transpose63_out1 = reshape57_out1.permute([0, 2, 1]);
        let concat121_out1 = burn::tensor::Tensor::cat(
            [mul171_out1.clone(), transpose63_out1.clone()].into(),
            2,
        );
        let linear68_out1 = self.linear68.forward(concat121_out1);
        let sigmoid38_out1 = burn::tensor::activation::sigmoid(linear68_out1);
        let shape153_out1: [i64; 3] = {
            let axes = &sigmoid38_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant1441_out1: [i64; 1] = [-1i64];
        let gather128_out1: [i64; 1usize] = constant1441_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape153_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape153_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant1443_out1: [i64; 1] = [1i64];
        let add97_out1 = {
            let __lhs = gather128_out1;
            let __rhs = constant1443_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant1444_out1: [i64; 1] = [2i64];
        let div107_out1 = {
            let __lhs = add97_out1;
            let __rhs = constant1444_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| if __rhs[__i] != 0 { __lhs[__i] / __rhs[__i] } else { __lhs[__i] })
        };
        let slice143_out1 = sigmoid38_out1.clone().slice(s![.., .., 0..div107_out1[0]]);
        let constant1446_out1: [i64; 1] = [2i64];
        let mul180_out1 = {
            let __lhs = div107_out1;
            let __rhs = constant1446_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice144_out1 = sigmoid38_out1
            .slice(s![.., .., div107_out1[0]..mul180_out1[0]]);
        let mul181_out1 = slice143_out1.mul(mul171_out1);
        let mul182_out1 = slice144_out1.mul(transpose63_out1);
        let add98_out1 = mul181_out1.add(mul182_out1);
        (add98_out1, constant339_out1, shape148_out1)
    }
}
#[derive(Module, Debug)]
pub struct Submodule19 {
    constant1447: burn::module::Param<Tensor<1>>,
    constant1448: burn::module::Param<Tensor<1>>,
    constant1449: burn::module::Param<Tensor<1>>,
    constant108: burn::module::Param<Tensor<1>>,
    linear69: Linear,
    linear70: Linear,
    constant1458: burn::module::Param<Tensor<1>>,
    constant1459: burn::module::Param<Tensor<1>>,
    constant1460: burn::module::Param<Tensor<1>>,
    constant111: burn::module::Param<Tensor<1>>,
    linear71: Linear,
    linear72: Linear,
    linear73: Linear,
    constant1464: burn::module::Param<Tensor<1>>,
    linear74: Linear,
    linear75: Linear,
    linear76: Linear,
    device: AllocationDevice,
}
impl Submodule19 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let constant1447: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1448: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.0000009999999974752427f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1449: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant108: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([192], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [192].into(),
        );
        let linear69 = LinearConfig::new(192, 512).with_bias(true).init(device);
        let linear70 = LinearConfig::new(256, 192).with_bias(true).init(device);
        let constant1458: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1459: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.0000009999999974752427f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1460: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant111: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([192], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [192].into(),
        );
        let linear71 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear72 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear73 = LinearConfig::new(192, 4).with_bias(true).init(device);
        let constant1464: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear74 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear75 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear76 = LinearConfig::new(192, 132).with_bias(true).init(device);
        Self {
            constant1447,
            constant1448,
            constant1449,
            constant108,
            linear69,
            linear70,
            constant1458,
            constant1459,
            constant1460,
            constant111,
            linear71,
            linear72,
            linear73,
            constant1464,
            linear74,
            linear75,
            linear76,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `add98_out1`: expected dtype `DType::F32`
    /// - `sigmoid35_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        add98_out1: Tensor<3>,
        sigmoid35_out1: Tensor<3>,
    ) -> ([i64; 3], Tensor<3>, Tensor<3>, Tensor<3>) {
        let constant1447_out1 = self.constant1447.val();
        let pow2_out1 = add98_out1
            .clone()
            .powf((constant1447_out1).unsqueeze_dims(&[0isize, 1isize]));
        let reducemean2_out1 = { pow2_out1.mean_dim(2usize) };
        let constant1448_out1 = self.constant1448.val();
        let add99_out1 = reducemean2_out1
            .add((constant1448_out1).unsqueeze_dims(&[0isize, 1isize]));
        let sqrt38_out1 = add99_out1.sqrt();
        let constant1449_out1 = self.constant1449.val();
        let div108_out1 = (constant1449_out1)
            .unsqueeze_dims(&[0isize, 1isize])
            .div(sqrt38_out1);
        let mul183_out1 = add98_out1.mul(div108_out1);
        let constant108_out1 = self.constant108.val();
        let mul184_out1 = mul183_out1
            .mul((constant108_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear69_out1 = self.linear69.forward(mul184_out1.clone());
        let shape154_out1: [i64; 3] = {
            let axes = &linear69_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant1450_out1: [i64; 1] = [-1i64];
        let gather129_out1: [i64; 1usize] = constant1450_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape154_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape154_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant1452_out1: [i64; 1] = [1i64];
        let add100_out1 = {
            let __lhs = gather129_out1;
            let __rhs = constant1452_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant1453_out1: [i64; 1] = [2i64];
        let div109_out1 = {
            let __lhs = add100_out1;
            let __rhs = constant1453_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| if __rhs[__i] != 0 { __lhs[__i] / __rhs[__i] } else { __lhs[__i] })
        };
        let slice145_out1 = linear69_out1.clone().slice(s![.., .., 0..div109_out1[0]]);
        let constant1455_out1: [i64; 1] = [2i64];
        let mul186_out1 = {
            let __lhs = div109_out1;
            let __rhs = constant1455_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice146_out1 = linear69_out1
            .slice(s![.., .., div109_out1[0]..mul186_out1[0]]);
        let sigmoid39_out1 = burn::tensor::activation::sigmoid(slice145_out1.clone());
        let mul187_out1 = slice145_out1.mul(sigmoid39_out1);
        let mul188_out1 = mul187_out1.mul(slice146_out1);
        let linear70_out1 = self.linear70.forward(mul188_out1);
        let add101_out1 = mul184_out1.add(linear70_out1);
        let clip2_out1 = add101_out1.clamp(-65504f64, 65504f64);
        let constant1458_out1 = self.constant1458.val();
        let pow3_out1 = clip2_out1
            .clone()
            .powf((constant1458_out1).unsqueeze_dims(&[0isize, 1isize]));
        let reducemean3_out1 = { pow3_out1.mean_dim(2usize) };
        let constant1459_out1 = self.constant1459.val();
        let add102_out1 = reducemean3_out1
            .add((constant1459_out1).unsqueeze_dims(&[0isize, 1isize]));
        let sqrt39_out1 = add102_out1.sqrt();
        let constant1460_out1 = self.constant1460.val();
        let div110_out1 = (constant1460_out1)
            .unsqueeze_dims(&[0isize, 1isize])
            .div(sqrt39_out1);
        let mul189_out1 = clip2_out1.mul(div110_out1);
        let constant111_out1 = self.constant111.val();
        let mul190_out1 = mul189_out1
            .mul((constant111_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear71_out1 = self.linear71.forward(mul190_out1.clone());
        let sigmoid40_out1 = burn::tensor::activation::sigmoid(linear71_out1.clone());
        let mul191_out1 = linear71_out1.mul(sigmoid40_out1);
        let linear72_out1 = self.linear72.forward(mul191_out1);
        let sigmoid41_out1 = burn::tensor::activation::sigmoid(linear72_out1.clone());
        let mul192_out1 = linear72_out1.mul(sigmoid41_out1);
        let linear73_out1 = self.linear73.forward(mul192_out1);
        let clip3_out1 = sigmoid35_out1.clamp(0f64, 1f64);
        let clip4_out1 = clip3_out1.clone().clamp_min(0.000009999999747378752f64);
        let constant1464_out1 = self.constant1464.val();
        let sub3_out1 = (constant1464_out1)
            .unsqueeze_dims(&[0isize, 1isize])
            .sub(clip3_out1);
        let clip5_out1 = sub3_out1.clamp_min(0.000009999999747378752f64);
        let div111_out1 = clip4_out1.div(clip5_out1);
        let log1_out1 = div111_out1.log();
        let add103_out1 = linear73_out1.add(log1_out1);
        let sigmoid42_out1 = burn::tensor::activation::sigmoid(add103_out1);
        let linear74_out1 = self.linear74.forward(mul190_out1.clone());
        let sigmoid43_out1 = burn::tensor::activation::sigmoid(linear74_out1.clone());
        let mul193_out1 = linear74_out1.mul(sigmoid43_out1);
        let linear75_out1 = self.linear75.forward(mul193_out1);
        let sigmoid44_out1 = burn::tensor::activation::sigmoid(linear75_out1.clone());
        let mul194_out1 = linear75_out1.mul(sigmoid44_out1);
        let linear76_out1 = self.linear76.forward(mul194_out1);
        let shape155_out1: [i64; 3] = {
            let axes = &linear76_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        (shape155_out1, linear76_out1, sigmoid42_out1, mul190_out1)
    }
}
#[derive(Module, Debug)]
pub struct Submodule20 {
    constant1469: burn::module::Param<Tensor<1>>,
    constant100: burn::module::Param<Tensor<1, Int>>,
    constant1474: burn::module::Param<Tensor<1>>,
    constant1484: burn::module::Param<Tensor<1>>,
    constant1485: burn::module::Param<Tensor<1>>,
    linear77: Linear,
    linear78: Linear,
    linear79: Linear,
    constant1507: burn::module::Param<Tensor<1>>,
    linear80: Linear,
    constant1514: burn::module::Param<Tensor<1>>,
    constant1515: burn::module::Param<Tensor<1>>,
    constant1516: burn::module::Param<Tensor<1>>,
    constant114: burn::module::Param<Tensor<1>>,
    linear81: Linear,
    linear82: Linear,
    constant1533: burn::module::Param<Tensor<1>>,
    constant1542: burn::module::Param<Tensor<1>>,
    constant1543: burn::module::Param<Tensor<1>>,
    linear83: Linear,
    device: AllocationDevice,
}
impl Submodule20 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let constant1469: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([33], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [33].into(),
        );
        let constant100: burn::module::Param<Tensor<1, Int>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
                Int,
            >::zeros([1], (device, burn::tensor::DType::I64)),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1474: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1484: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1485: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear77 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear78 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear79 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let constant1507: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.20412415266036987f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear80 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let constant1514: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1515: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.0000009999999974752427f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1516: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant114: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([192], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [192].into(),
        );
        let linear81 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear82 = LinearConfig::new(192, 96).with_bias(true).init(device);
        let constant1533: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1542: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1543: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear83 = LinearConfig::new(384, 384).with_bias(true).init(device);
        Self {
            constant1469,
            constant100,
            constant1474,
            constant1484,
            constant1485,
            linear77,
            linear78,
            linear79,
            constant1507,
            linear80,
            constant1514,
            constant1515,
            constant1516,
            constant114,
            linear81,
            linear82,
            constant1533,
            constant1542,
            constant1543,
            linear83,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `linear76_out1`: expected dtype `DType::F32`
    /// - `sigmoid42_out1`: expected dtype `DType::F32`
    /// - `mul190_out1`: expected dtype `DType::F32`
    /// - `clip1_out1`: expected dtype `DType::F32`
    /// - `constant339_out1`: expected dtype `DType::F32`
    /// - `slice136_out1`: expected dtype `DType::F32`
    /// - `slice137_out1`: expected dtype `DType::F32`
    /// - `slice138_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        shape155_out1: [i64; 3],
        linear76_out1: Tensor<3>,
        sigmoid42_out1: Tensor<3>,
        mul190_out1: Tensor<3>,
        clip1_out1: Tensor<3>,
        constant339_out1: Tensor<2>,
        shape148_out1: [i64; 4],
        gather107_out1: i64,
        gather108_out1: i64,
        slice136_out1: Tensor<4>,
        gather109_out1: i64,
        gather110_out1: i64,
        slice137_out1: Tensor<4>,
        gather111_out1: i64,
        gather112_out1: i64,
        slice138_out1: Tensor<4>,
    ) -> (
        Tensor<3>,
        Tensor<1>,
        Tensor<1, Int>,
        Tensor<2>,
        Tensor<2>,
        Tensor<2>,
        Tensor<2>,
    ) {
        let gather130_out1 = shape155_out1[0] as i64;
        let gather131_out1 = shape155_out1[1] as i64;
        let reshape58_out1 = linear76_out1
            .reshape(burn::tensor::ZeroDimPlaceholders([-1, 33]));
        let softmax16_out1 = burn::tensor::activation::softmax(reshape58_out1, 1);
        let constant1469_out1 = self.constant1469.val();
        let matmul103_out1 = softmax16_out1
            .matmul(constant1469_out1.clone().unsqueeze_dims(&[-1isize]))
            .squeeze_dim::<1usize>(1usize);
        let reshape59_out1 = matmul103_out1
            .reshape(burn::tensor::ZeroDimPlaceholders([-1, 4]));
        let unsqueeze199_out1 = [gather130_out1 as i64];
        let unsqueeze200_out1 = [gather131_out1 as i64];
        let constant1473_out1: [i64; 1] = [-1i64];
        let concat122_out1: [i64; 3usize] = [
            &unsqueeze199_out1[..],
            &unsqueeze200_out1[..],
            &constant1473_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape60_out1 = reshape59_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat122_out1));
        let constant100_out1 = self.constant100.val();
        let abs1_out1 = constant100_out1.clone().abs();
        let gather132_out1 = {
            let sliced = sigmoid42_out1.clone().slice(s![.., .., 0]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let cast164_out1 = abs1_out1.float().cast(burn::tensor::DType::F32);
        let constant1474_out1 = self.constant1474.val();
        let mul195_out1 = cast164_out1.clone().mul(constant1474_out1);
        let gather133_out1 = {
            let sliced = reshape60_out1.clone().slice(s![.., .., 0]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let add104_out1 = (mul195_out1.clone())
            .unsqueeze_dims(&[0isize])
            .add(gather133_out1);
        let gather134_out1 = {
            let sliced = sigmoid42_out1.clone().slice(s![.., .., 2]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let div112_out1 = gather134_out1
            .clone()
            .div((cast164_out1.clone()).unsqueeze_dims(&[0isize]));
        let mul196_out1 = add104_out1.mul(div112_out1.clone());
        let sub4_out1 = gather132_out1.clone().sub(mul196_out1);
        let gather135_out1 = {
            let sliced = sigmoid42_out1.clone().slice(s![.., .., 1]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let gather136_out1 = {
            let sliced = reshape60_out1.clone().slice(s![.., .., 1]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let add105_out1 = (mul195_out1.clone())
            .unsqueeze_dims(&[0isize])
            .add(gather136_out1);
        let gather137_out1 = {
            let sliced = sigmoid42_out1.slice(s![.., .., 3]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let div113_out1 = gather137_out1
            .clone()
            .div((cast164_out1).unsqueeze_dims(&[0isize]));
        let mul197_out1 = add105_out1.mul(div113_out1.clone());
        let sub5_out1 = gather135_out1.clone().sub(mul197_out1);
        let gather138_out1 = {
            let sliced = reshape60_out1.clone().slice(s![.., .., 2]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let add106_out1 = (mul195_out1.clone())
            .unsqueeze_dims(&[0isize])
            .add(gather138_out1);
        let mul198_out1 = add106_out1.mul(div112_out1);
        let add107_out1 = gather132_out1.clone().add(mul198_out1);
        let gather139_out1 = {
            let sliced = reshape60_out1.slice(s![.., .., 3]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let add108_out1 = (mul195_out1).unsqueeze_dims(&[0isize]).add(gather139_out1);
        let mul199_out1 = add108_out1.mul(div113_out1);
        let add109_out1 = gather135_out1.clone().add(mul199_out1);
        let unsqueeze201_out1: Tensor<3> = sub4_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze202_out1: Tensor<3> = sub5_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze203_out1: Tensor<3> = add107_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze204_out1: Tensor<3> = add109_out1.unsqueeze_dims::<3>(&[-1]);
        let concat123_out1 = burn::tensor::Tensor::cat(
            [unsqueeze201_out1, unsqueeze202_out1, unsqueeze203_out1, unsqueeze204_out1]
                .into(),
            2,
        );
        let [split18_out1, split18_out2, split18_out3, split18_out4] = {
            let split_tensors = concat123_out1.split_with_sizes([1, 1, 1, 1].into(), 2);
            split_tensors.try_into().unwrap()
        };
        let squeeze37_out1 = split18_out1.squeeze_dims::<2>(&[-1]);
        let squeeze38_out1 = split18_out2.squeeze_dims::<2>(&[-1]);
        let squeeze39_out1 = split18_out3.squeeze_dims::<2>(&[-1]);
        let squeeze40_out1 = split18_out4.squeeze_dims::<2>(&[-1]);
        let add110_out1 = squeeze37_out1.clone().add(squeeze39_out1.clone());
        let constant1484_out1 = self.constant1484.val();
        let div114_out1 = add110_out1.div((constant1484_out1).unsqueeze_dims(&[0isize]));
        let add111_out1 = squeeze38_out1.clone().add(squeeze40_out1.clone());
        let constant1485_out1 = self.constant1485.val();
        let div115_out1 = add111_out1.div((constant1485_out1).unsqueeze_dims(&[0isize]));
        let sub6_out1 = squeeze39_out1.sub(squeeze37_out1);
        let sub7_out1 = squeeze40_out1.sub(squeeze38_out1);
        let unsqueeze205_out1: Tensor<3> = div114_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze206_out1: Tensor<3> = div115_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze207_out1: Tensor<3> = sub6_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze208_out1: Tensor<3> = sub7_out1.unsqueeze_dims::<3>(&[-1]);
        let concat124_out1 = burn::tensor::Tensor::cat(
            [unsqueeze205_out1, unsqueeze206_out1, unsqueeze207_out1, unsqueeze208_out1]
                .into(),
            2,
        );
        let unsqueeze209_out1: Tensor<4> = concat124_out1.unsqueeze_dims::<4>(&[2]);
        let add112_out1 = mul190_out1.clone().add(clip1_out1.clone());
        let transpose64_out1 = add112_out1.permute([1, 0, 2]);
        let transpose65_out1 = mul190_out1.clone().permute([1, 0, 2]);
        let shape157_out1: [i64; 3] = {
            let axes = &transpose64_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather140_out1 = shape157_out1[0] as i64;
        let gather141_out1 = shape157_out1[1] as i64;
        let gather142_out1 = shape157_out1[2] as i64;
        let constant1494_out1 = 8i64;
        let div116_out1 = gather142_out1 / constant1494_out1;
        let linear77_out1 = self.linear77.forward(transpose64_out1.clone());
        let linear78_out1 = self.linear78.forward(transpose64_out1);
        let linear79_out1 = self.linear79.forward(transpose65_out1);
        let constant1495_out1 = 8i64;
        let mul200_out1 = gather141_out1 * constant1495_out1;
        let unsqueeze210_out1 = [gather140_out1 as i64];
        let unsqueeze211_out1 = [mul200_out1 as i64];
        let unsqueeze212_out1 = [div116_out1 as i64];
        let concat125_out1: [i64; 3usize] = [
            &unsqueeze210_out1[..],
            &unsqueeze211_out1[..],
            &unsqueeze212_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape61_out1 = linear77_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat125_out1));
        let transpose66_out1 = reshape61_out1.permute([1, 0, 2]);
        let shape160_out1: [i64; 3] = {
            let axes = &linear78_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather143_out1 = shape160_out1[0] as i64;
        let unsqueeze213_out1 = [gather143_out1 as i64];
        let unsqueeze214_out1 = [mul200_out1 as i64];
        let unsqueeze215_out1 = [div116_out1 as i64];
        let concat126_out1: [i64; 3usize] = [
            &unsqueeze213_out1[..],
            &unsqueeze214_out1[..],
            &unsqueeze215_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape62_out1 = linear78_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat126_out1));
        let shape161_out1: [i64; 3] = {
            let axes = &linear79_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather144_out1 = shape161_out1[0] as i64;
        let unsqueeze216_out1 = [gather144_out1 as i64];
        let unsqueeze217_out1 = [mul200_out1 as i64];
        let unsqueeze218_out1 = [div116_out1 as i64];
        let concat127_out1: [i64; 3usize] = [
            &unsqueeze216_out1[..],
            &unsqueeze217_out1[..],
            &unsqueeze218_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape63_out1 = linear79_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat127_out1));
        let transpose67_out1 = reshape63_out1.permute([1, 0, 2]);
        let constant1507_out1 = self.constant1507.val();
        let mul201_out1 = transpose66_out1
            .mul((constant1507_out1).unsqueeze_dims(&[0isize, 1isize]));
        let transpose68_out1 = reshape62_out1.permute([1, 2, 0]);
        let matmul107_out1 = mul201_out1.matmul(transpose68_out1);
        let softmax17_out1 = burn::tensor::activation::softmax(matmul107_out1, 2);
        let matmul108_out1 = softmax17_out1.matmul(transpose67_out1);
        let transpose69_out1 = matmul108_out1.permute([1, 0, 2]);
        let mul202_out1 = gather140_out1 * gather141_out1;
        let unsqueeze219_out1 = [mul202_out1 as i64];
        let unsqueeze220_out1 = [gather142_out1 as i64];
        let concat128_out1: [i64; 2usize] = [
            &unsqueeze219_out1[..],
            &unsqueeze220_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape64_out1 = transpose69_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat128_out1));
        let linear80_out1 = self.linear80.forward(reshape64_out1);
        let shape162_out1: [i64; 2] = {
            let axes = &linear80_out1.clone().dims()[0..2];
            let mut output = [0i64; 2];
            for i in 0..2 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather145_out1 = shape162_out1[1] as i64;
        let unsqueeze221_out1 = [gather140_out1 as i64];
        let unsqueeze222_out1 = [gather141_out1 as i64];
        let unsqueeze223_out1 = [gather145_out1 as i64];
        let concat129_out1: [i64; 3usize] = [
            &unsqueeze221_out1[..],
            &unsqueeze222_out1[..],
            &unsqueeze223_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape65_out1 = linear80_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat129_out1));
        let transpose70_out1 = reshape65_out1.permute([1, 0, 2]);
        let add113_out1 = mul190_out1.add(transpose70_out1);
        let constant1514_out1 = self.constant1514.val();
        let pow4_out1 = add113_out1
            .clone()
            .powf((constant1514_out1).unsqueeze_dims(&[0isize, 1isize]));
        let reducemean4_out1 = { pow4_out1.mean_dim(2usize) };
        let constant1515_out1 = self.constant1515.val();
        let add114_out1 = reducemean4_out1
            .add((constant1515_out1).unsqueeze_dims(&[0isize, 1isize]));
        let sqrt40_out1 = add114_out1.sqrt();
        let constant1516_out1 = self.constant1516.val();
        let div117_out1 = (constant1516_out1)
            .unsqueeze_dims(&[0isize, 1isize])
            .div(sqrt40_out1);
        let mul203_out1 = add113_out1.mul(div117_out1);
        let constant114_out1 = self.constant114.val();
        let mul204_out1 = mul203_out1
            .mul((constant114_out1).unsqueeze_dims(&[0isize, 1isize]));
        let add115_out1 = mul204_out1.clone().add(clip1_out1);
        let shape163_out1: [i64; 3] = {
            let axes = &add115_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather146_out1 = shape163_out1[0] as i64;
        let gather147_out1 = shape163_out1[1] as i64;
        let linear81_out1 = self.linear81.forward(add115_out1.clone());
        let unsqueeze224_out1 = [gather146_out1 as i64];
        let unsqueeze225_out1 = [gather147_out1 as i64];
        let constant1523_out1: [i64; 1] = [2i64];
        let constant1521_out1: [i64; 1] = [8i64];
        let constant1522_out1: [i64; 1] = [12i64];
        let concat130_out1: [i64; 5usize] = [
            &unsqueeze224_out1[..],
            &unsqueeze225_out1[..],
            &constant1521_out1[..],
            &constant1522_out1[..],
            &constant1523_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape66_out1 = linear81_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat130_out1));
        let linear82_out1 = self.linear82.forward(add115_out1);
        let unsqueeze226_out1 = [gather146_out1 as i64];
        let unsqueeze227_out1 = [gather147_out1 as i64];
        let constant1527_out1: [i64; 1] = [12i64];
        let constant1526_out1: [i64; 1] = [8i64];
        let concat131_out1: [i64; 4usize] = [
            &unsqueeze226_out1[..],
            &unsqueeze227_out1[..],
            &constant1526_out1[..],
            &constant1527_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape67_out1 = linear82_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat131_out1));
        let softmax18_out1 = burn::tensor::activation::softmax(reshape67_out1, 3);
        let mul205_out1 = reshape66_out1
            .mul((constant339_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let unsqueeze228_out1: Tensor<5> = unsqueeze209_out1.unsqueeze_dims::<5>(&[2]);
        let slice147_out1 = unsqueeze228_out1.clone().slice(s![.., .., .., .., 2..]);
        let mul206_out1 = mul205_out1.mul(slice147_out1);
        let constant1533_out1 = self.constant1533.val();
        let mul207_out1 = mul206_out1
            .mul((constant1533_out1).unsqueeze_dims(&[0isize, 1isize, 2isize, 3isize]));
        let slice148_out1 = unsqueeze228_out1.slice(s![.., .., .., .., 0..2]);
        let add116_out1 = slice148_out1.add(mul207_out1);
        let gather148_out1 = shape148_out1[0] as i64;
        let gather149_out1 = shape148_out1[1] as i64;
        let gather150_out1 = shape148_out1[2] as i64;
        let shape168_out1: [i64; 5] = {
            let axes = &add116_out1.clone().dims()[0..5];
            let mut output = [0i64; 5];
            for i in 0..5 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather151_out1 = shape168_out1[1] as i64;
        let constant1542_out1 = self.constant1542.val();
        let mul208_out1 = add116_out1
            .mul((constant1542_out1).unsqueeze_dims(&[0isize, 1isize, 2isize, 3isize]));
        let constant1543_out1 = self.constant1543.val();
        let sub8_out1 = mul208_out1
            .sub((constant1543_out1).unsqueeze_dims(&[0isize, 1isize, 2isize, 3isize]));
        let transpose71_out1 = sub8_out1.permute([0, 2, 1, 3, 4]);
        let shape169_out1: [i64; 5] = {
            let axes = &transpose71_out1.clone().dims()[0..5];
            let mut output = [0i64; 5];
            for i in 0..5 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice149_out1: [i64; 0] = shape169_out1[0..0].try_into().unwrap();
        let slice150_out1: [i64; 3] = shape169_out1[2..5].try_into().unwrap();
        let constant1550_out1: [i64; 1] = [-1i64];
        let concat132_out1: [i64; 4usize] = [
            &slice149_out1[..],
            &constant1550_out1[..],
            &slice150_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape68_out1 = transpose71_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat132_out1));
        let [split19_out1, split19_out2, split19_out3] = {
            let split_tensors = reshape68_out1.split_with_sizes([3, 6, 3].into(), 2);
            split_tensors.try_into().unwrap()
        };
        let mul209_out1 = gather148_out1 * gather149_out1;
        let unsqueeze229_out1 = [mul209_out1 as i64];
        let unsqueeze230_out1 = [gather150_out1 as i64];
        let unsqueeze231_out1 = [gather107_out1 as i64];
        let unsqueeze232_out1 = [gather108_out1 as i64];
        let concat133_out1: [i64; 4usize] = [
            &unsqueeze229_out1[..],
            &unsqueeze230_out1[..],
            &unsqueeze231_out1[..],
            &unsqueeze232_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape69_out1 = slice136_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat133_out1));
        let gridsample4_out1 = reshape69_out1
            .grid_sample_2d(
                split19_out1,
                burn::tensor::ops::GridSampleOptions::new(
                        burn::tensor::ops::GridSampleMode::Bilinear,
                    )
                    .with_padding_mode(burn::tensor::ops::GridSamplePaddingMode::Zeros)
                    .with_align_corners(false),
            );
        let unsqueeze233_out1 = [mul209_out1 as i64];
        let unsqueeze234_out1 = [gather150_out1 as i64];
        let unsqueeze235_out1 = [gather109_out1 as i64];
        let unsqueeze236_out1 = [gather110_out1 as i64];
        let concat134_out1: [i64; 4usize] = [
            &unsqueeze233_out1[..],
            &unsqueeze234_out1[..],
            &unsqueeze235_out1[..],
            &unsqueeze236_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape70_out1 = slice137_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat134_out1));
        let gridsample5_out1 = reshape70_out1
            .grid_sample_2d(
                split19_out2,
                burn::tensor::ops::GridSampleOptions::new(
                        burn::tensor::ops::GridSampleMode::Bilinear,
                    )
                    .with_padding_mode(burn::tensor::ops::GridSamplePaddingMode::Zeros)
                    .with_align_corners(false),
            );
        let unsqueeze237_out1 = [mul209_out1 as i64];
        let unsqueeze238_out1 = [gather150_out1 as i64];
        let unsqueeze239_out1 = [gather111_out1 as i64];
        let unsqueeze240_out1 = [gather112_out1 as i64];
        let concat135_out1: [i64; 4usize] = [
            &unsqueeze237_out1[..],
            &unsqueeze238_out1[..],
            &unsqueeze239_out1[..],
            &unsqueeze240_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape71_out1 = slice138_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat135_out1));
        let gridsample6_out1 = reshape71_out1
            .grid_sample_2d(
                split19_out3,
                burn::tensor::ops::GridSampleOptions::new(
                        burn::tensor::ops::GridSampleMode::Bilinear,
                    )
                    .with_padding_mode(burn::tensor::ops::GridSamplePaddingMode::Zeros)
                    .with_align_corners(false),
            );
        let transpose72_out1 = softmax18_out1.permute([0, 2, 1, 3]);
        let unsqueeze241_out1 = [mul209_out1 as i64];
        let unsqueeze242_out1 = [gather151_out1 as i64];
        let constant1566_out1: [i64; 1] = [12i64];
        let constant1564_out1: [i64; 1] = [1i64];
        let concat136_out1: [i64; 4usize] = [
            &unsqueeze241_out1[..],
            &constant1564_out1[..],
            &unsqueeze242_out1[..],
            &constant1566_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape72_out1 = transpose72_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat136_out1));
        let concat137_out1 = burn::tensor::Tensor::cat(
            [gridsample4_out1, gridsample5_out1, gridsample6_out1].into(),
            3,
        );
        let mul210_out1 = concat137_out1.mul(reshape72_out1);
        let reducesum2_out1 = {
            mul210_out1.sum_dim(3usize).squeeze_dims::<3usize>(&[3])
        };
        let mul211_out1 = gather149_out1 * gather150_out1;
        let unsqueeze243_out1 = [gather148_out1 as i64];
        let unsqueeze244_out1 = [mul211_out1 as i64];
        let unsqueeze245_out1 = [gather151_out1 as i64];
        let concat138_out1: [i64; 3usize] = [
            &unsqueeze243_out1[..],
            &unsqueeze244_out1[..],
            &unsqueeze245_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape73_out1 = reducesum2_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat138_out1));
        let transpose73_out1 = reshape73_out1.permute([0, 2, 1]);
        let concat139_out1 = burn::tensor::Tensor::cat(
            [mul204_out1.clone(), transpose73_out1.clone()].into(),
            2,
        );
        let linear83_out1 = self.linear83.forward(concat139_out1);
        let sigmoid45_out1 = burn::tensor::activation::sigmoid(linear83_out1);
        let shape170_out1: [i64; 3] = {
            let axes = &sigmoid45_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant1570_out1: [i64; 1] = [-1i64];
        let gather152_out1: [i64; 1usize] = constant1570_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape170_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape170_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant1572_out1: [i64; 1] = [1i64];
        let add117_out1 = {
            let __lhs = gather152_out1;
            let __rhs = constant1572_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant1573_out1: [i64; 1] = [2i64];
        let div118_out1 = {
            let __lhs = add117_out1;
            let __rhs = constant1573_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| if __rhs[__i] != 0 { __lhs[__i] / __rhs[__i] } else { __lhs[__i] })
        };
        let slice151_out1 = sigmoid45_out1.clone().slice(s![.., .., 0..div118_out1[0]]);
        let constant1575_out1: [i64; 1] = [2i64];
        let mul213_out1 = {
            let __lhs = div118_out1;
            let __rhs = constant1575_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice152_out1 = sigmoid45_out1
            .slice(s![.., .., div118_out1[0]..mul213_out1[0]]);
        let mul214_out1 = slice151_out1.mul(mul204_out1);
        let mul215_out1 = slice152_out1.mul(transpose73_out1);
        let add118_out1 = mul214_out1.add(mul215_out1);
        (
            add118_out1,
            constant1469_out1,
            constant100_out1,
            gather134_out1,
            gather132_out1,
            gather137_out1,
            gather135_out1,
        )
    }
}
#[derive(Module, Debug)]
pub struct Submodule21 {
    constant1576: burn::module::Param<Tensor<1>>,
    constant1577: burn::module::Param<Tensor<1>>,
    constant1578: burn::module::Param<Tensor<1>>,
    constant118: burn::module::Param<Tensor<1>>,
    linear84: Linear,
    linear85: Linear,
    constant1587: burn::module::Param<Tensor<1>>,
    constant1588: burn::module::Param<Tensor<1>>,
    constant1589: burn::module::Param<Tensor<1>>,
    constant121: burn::module::Param<Tensor<1>>,
    linear86: Linear,
    linear87: Linear,
    linear88: Linear,
    constant1597: burn::module::Param<Tensor<1>>,
    device: AllocationDevice,
}
impl Submodule21 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let constant1576: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1577: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.0000009999999974752427f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1578: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant118: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([192], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [192].into(),
        );
        let linear84 = LinearConfig::new(192, 512).with_bias(true).init(device);
        let linear85 = LinearConfig::new(256, 192).with_bias(true).init(device);
        let constant1587: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1588: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.0000009999999974752427f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1589: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant121: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([192], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [192].into(),
        );
        let linear86 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear87 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear88 = LinearConfig::new(192, 132).with_bias(true).init(device);
        let constant1597: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        Self {
            constant1576,
            constant1577,
            constant1578,
            constant118,
            linear84,
            linear85,
            constant1587,
            constant1588,
            constant1589,
            constant121,
            linear86,
            linear87,
            linear88,
            constant1597,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `add118_out1`: expected dtype `DType::F32`
    /// - `mul190_out1`: expected dtype `DType::F32`
    /// - `linear76_out1`: expected dtype `DType::F32`
    /// - `constant1469_out1`: expected dtype `DType::F32`
    /// - `constant100_out1`: expected dtype `DType::I64`
    /// - `gather134_out1`: expected dtype `DType::F32`
    /// - `gather132_out1`: expected dtype `DType::F32`
    /// - `gather137_out1`: expected dtype `DType::F32`
    /// - `gather135_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        add118_out1: Tensor<3>,
        mul190_out1: Tensor<3>,
        linear76_out1: Tensor<3>,
        constant1469_out1: Tensor<1>,
        constant100_out1: Tensor<1, Int>,
        gather134_out1: Tensor<2>,
        gather132_out1: Tensor<2>,
        gather137_out1: Tensor<2>,
        gather135_out1: Tensor<2>,
    ) -> (Tensor<3>, Tensor<3>, Tensor<3>) {
        let constant1576_out1 = self.constant1576.val();
        let pow5_out1 = add118_out1
            .clone()
            .powf((constant1576_out1).unsqueeze_dims(&[0isize, 1isize]));
        let reducemean5_out1 = { pow5_out1.mean_dim(2usize) };
        let constant1577_out1 = self.constant1577.val();
        let add119_out1 = reducemean5_out1
            .add((constant1577_out1).unsqueeze_dims(&[0isize, 1isize]));
        let sqrt41_out1 = add119_out1.sqrt();
        let constant1578_out1 = self.constant1578.val();
        let div119_out1 = (constant1578_out1)
            .unsqueeze_dims(&[0isize, 1isize])
            .div(sqrt41_out1);
        let mul216_out1 = add118_out1.mul(div119_out1);
        let constant118_out1 = self.constant118.val();
        let mul217_out1 = mul216_out1
            .mul((constant118_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear84_out1 = self.linear84.forward(mul217_out1.clone());
        let shape171_out1: [i64; 3] = {
            let axes = &linear84_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant1579_out1: [i64; 1] = [-1i64];
        let gather153_out1: [i64; 1usize] = constant1579_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape171_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape171_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant1581_out1: [i64; 1] = [1i64];
        let add120_out1 = {
            let __lhs = gather153_out1;
            let __rhs = constant1581_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant1582_out1: [i64; 1] = [2i64];
        let div120_out1 = {
            let __lhs = add120_out1;
            let __rhs = constant1582_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| if __rhs[__i] != 0 { __lhs[__i] / __rhs[__i] } else { __lhs[__i] })
        };
        let slice153_out1 = linear84_out1.clone().slice(s![.., .., 0..div120_out1[0]]);
        let constant1584_out1: [i64; 1] = [2i64];
        let mul219_out1 = {
            let __lhs = div120_out1;
            let __rhs = constant1584_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice154_out1 = linear84_out1
            .slice(s![.., .., div120_out1[0]..mul219_out1[0]]);
        let sigmoid46_out1 = burn::tensor::activation::sigmoid(slice153_out1.clone());
        let mul220_out1 = slice153_out1.mul(sigmoid46_out1);
        let mul221_out1 = mul220_out1.mul(slice154_out1);
        let linear85_out1 = self.linear85.forward(mul221_out1);
        let add121_out1 = mul217_out1.add(linear85_out1);
        let clip6_out1 = add121_out1.clamp(-65504f64, 65504f64);
        let constant1587_out1 = self.constant1587.val();
        let pow6_out1 = clip6_out1
            .clone()
            .powf((constant1587_out1).unsqueeze_dims(&[0isize, 1isize]));
        let reducemean6_out1 = { pow6_out1.mean_dim(2usize) };
        let constant1588_out1 = self.constant1588.val();
        let add122_out1 = reducemean6_out1
            .add((constant1588_out1).unsqueeze_dims(&[0isize, 1isize]));
        let sqrt42_out1 = add122_out1.sqrt();
        let constant1589_out1 = self.constant1589.val();
        let div121_out1 = (constant1589_out1)
            .unsqueeze_dims(&[0isize, 1isize])
            .div(sqrt42_out1);
        let mul222_out1 = clip6_out1.mul(div121_out1);
        let constant121_out1 = self.constant121.val();
        let mul223_out1 = mul222_out1
            .mul((constant121_out1).unsqueeze_dims(&[0isize, 1isize]));
        let add123_out1 = mul223_out1.clone().add(mul190_out1);
        let linear86_out1 = self.linear86.forward(add123_out1);
        let sigmoid47_out1 = burn::tensor::activation::sigmoid(linear86_out1.clone());
        let mul224_out1 = linear86_out1.mul(sigmoid47_out1);
        let linear87_out1 = self.linear87.forward(mul224_out1);
        let sigmoid48_out1 = burn::tensor::activation::sigmoid(linear87_out1.clone());
        let mul225_out1 = linear87_out1.mul(sigmoid48_out1);
        let linear88_out1 = self.linear88.forward(mul225_out1);
        let add124_out1 = linear88_out1.add(linear76_out1);
        let shape172_out1: [i64; 3] = {
            let axes = &add124_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather154_out1 = shape172_out1[0] as i64;
        let gather155_out1 = shape172_out1[1] as i64;
        let reshape74_out1 = add124_out1
            .clone()
            .reshape(burn::tensor::ZeroDimPlaceholders([-1, 33]));
        let softmax19_out1 = burn::tensor::activation::softmax(reshape74_out1, 1);
        let matmul117_out1 = softmax19_out1
            .matmul(constant1469_out1.unsqueeze_dims(&[-1isize]))
            .squeeze_dim::<1usize>(1usize);
        let reshape75_out1 = matmul117_out1
            .reshape(burn::tensor::ZeroDimPlaceholders([-1, 4]));
        let unsqueeze246_out1 = [gather154_out1 as i64];
        let unsqueeze247_out1 = [gather155_out1 as i64];
        let constant1596_out1: [i64; 1] = [-1i64];
        let concat140_out1: [i64; 3usize] = [
            &unsqueeze246_out1[..],
            &unsqueeze247_out1[..],
            &constant1596_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape76_out1 = reshape75_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat140_out1));
        let abs2_out1 = constant100_out1.abs();
        let cast172_out1 = abs2_out1.float().cast(burn::tensor::DType::F32);
        let constant1597_out1 = self.constant1597.val();
        let mul226_out1 = cast172_out1.clone().mul(constant1597_out1);
        let gather156_out1 = {
            let sliced = reshape76_out1.clone().slice(s![.., .., 0]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let add125_out1 = (mul226_out1.clone())
            .unsqueeze_dims(&[0isize])
            .add(gather156_out1);
        let div122_out1 = gather134_out1
            .div((cast172_out1.clone()).unsqueeze_dims(&[0isize]));
        let mul227_out1 = add125_out1.mul(div122_out1.clone());
        let sub9_out1 = gather132_out1.clone().sub(mul227_out1);
        let gather157_out1 = {
            let sliced = reshape76_out1.clone().slice(s![.., .., 1]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let add126_out1 = (mul226_out1.clone())
            .unsqueeze_dims(&[0isize])
            .add(gather157_out1);
        let div123_out1 = gather137_out1.div((cast172_out1).unsqueeze_dims(&[0isize]));
        let mul228_out1 = add126_out1.mul(div123_out1.clone());
        let sub10_out1 = gather135_out1.clone().sub(mul228_out1);
        let gather158_out1 = {
            let sliced = reshape76_out1.clone().slice(s![.., .., 2]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let add127_out1 = (mul226_out1.clone())
            .unsqueeze_dims(&[0isize])
            .add(gather158_out1);
        let mul229_out1 = add127_out1.mul(div122_out1);
        let add128_out1 = gather132_out1.add(mul229_out1);
        let gather159_out1 = {
            let sliced = reshape76_out1.slice(s![.., .., 3]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let add129_out1 = (mul226_out1).unsqueeze_dims(&[0isize]).add(gather159_out1);
        let mul230_out1 = add129_out1.mul(div123_out1);
        let add130_out1 = gather135_out1.add(mul230_out1);
        let unsqueeze248_out1: Tensor<3> = sub9_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze249_out1: Tensor<3> = sub10_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze250_out1: Tensor<3> = add128_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze251_out1: Tensor<3> = add130_out1.unsqueeze_dims::<3>(&[-1]);
        let concat141_out1 = burn::tensor::Tensor::cat(
            [unsqueeze248_out1, unsqueeze249_out1, unsqueeze250_out1, unsqueeze251_out1]
                .into(),
            2,
        );
        (concat141_out1, mul223_out1, add124_out1)
    }
}
#[derive(Module, Debug)]
pub struct Submodule22 {
    constant1607: burn::module::Param<Tensor<1>>,
    constant1608: burn::module::Param<Tensor<1>>,
    linear89: Linear,
    linear90: Linear,
    linear91: Linear,
    constant1630: burn::module::Param<Tensor<1>>,
    linear92: Linear,
    constant1637: burn::module::Param<Tensor<1>>,
    constant1638: burn::module::Param<Tensor<1>>,
    constant1639: burn::module::Param<Tensor<1>>,
    constant124: burn::module::Param<Tensor<1>>,
    linear93: Linear,
    linear94: Linear,
    constant1656: burn::module::Param<Tensor<1>>,
    constant1665: burn::module::Param<Tensor<1>>,
    constant1666: burn::module::Param<Tensor<1>>,
    linear95: Linear,
    constant1699: burn::module::Param<Tensor<1>>,
    constant1700: burn::module::Param<Tensor<1>>,
    constant1701: burn::module::Param<Tensor<1>>,
    constant128: burn::module::Param<Tensor<1>>,
    linear96: Linear,
    linear97: Linear,
    constant1710: burn::module::Param<Tensor<1>>,
    constant1711: burn::module::Param<Tensor<1>>,
    constant1712: burn::module::Param<Tensor<1>>,
    constant131: burn::module::Param<Tensor<1>>,
    linear98: Linear,
    linear99: Linear,
    linear100: Linear,
    device: AllocationDevice,
}
impl Submodule22 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let constant1607: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1608: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear89 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear90 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear91 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let constant1630: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.20412415266036987f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear92 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let constant1637: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1638: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.0000009999999974752427f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1639: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant124: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([192], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [192].into(),
        );
        let linear93 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear94 = LinearConfig::new(192, 96).with_bias(true).init(device);
        let constant1656: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1665: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1666: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear95 = LinearConfig::new(384, 384).with_bias(true).init(device);
        let constant1699: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1700: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.0000009999999974752427f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1701: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant128: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([192], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [192].into(),
        );
        let linear96 = LinearConfig::new(192, 512).with_bias(true).init(device);
        let linear97 = LinearConfig::new(256, 192).with_bias(true).init(device);
        let constant1710: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1711: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.0000009999999974752427f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1712: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant131: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([192], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [192].into(),
        );
        let linear98 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear99 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear100 = LinearConfig::new(192, 132).with_bias(true).init(device);
        Self {
            constant1607,
            constant1608,
            linear89,
            linear90,
            linear91,
            constant1630,
            linear92,
            constant1637,
            constant1638,
            constant1639,
            constant124,
            linear93,
            linear94,
            constant1656,
            constant1665,
            constant1666,
            linear95,
            constant1699,
            constant1700,
            constant1701,
            constant128,
            linear96,
            linear97,
            constant1710,
            constant1711,
            constant1712,
            constant131,
            linear98,
            linear99,
            linear100,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `concat141_out1`: expected dtype `DType::F32`
    /// - `mul223_out1`: expected dtype `DType::F32`
    /// - `clip1_out1`: expected dtype `DType::F32`
    /// - `constant339_out1`: expected dtype `DType::F32`
    /// - `slice136_out1`: expected dtype `DType::F32`
    /// - `slice137_out1`: expected dtype `DType::F32`
    /// - `slice138_out1`: expected dtype `DType::F32`
    /// - `add124_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        concat141_out1: Tensor<3>,
        mul223_out1: Tensor<3>,
        clip1_out1: Tensor<3>,
        constant339_out1: Tensor<2>,
        shape148_out1: [i64; 4],
        gather107_out1: i64,
        gather108_out1: i64,
        slice136_out1: Tensor<4>,
        gather109_out1: i64,
        gather110_out1: i64,
        slice137_out1: Tensor<4>,
        gather111_out1: i64,
        gather112_out1: i64,
        slice138_out1: Tensor<4>,
        add124_out1: Tensor<3>,
    ) -> (Tensor<3>, Tensor<3>) {
        let [split20_out1, split20_out2, split20_out3, split20_out4] = {
            let split_tensors = concat141_out1.split_with_sizes([1, 1, 1, 1].into(), 2);
            split_tensors.try_into().unwrap()
        };
        let squeeze41_out1 = split20_out1.squeeze_dims::<2>(&[-1]);
        let squeeze42_out1 = split20_out2.squeeze_dims::<2>(&[-1]);
        let squeeze43_out1 = split20_out3.squeeze_dims::<2>(&[-1]);
        let squeeze44_out1 = split20_out4.squeeze_dims::<2>(&[-1]);
        let add131_out1 = squeeze41_out1.clone().add(squeeze43_out1.clone());
        let constant1607_out1 = self.constant1607.val();
        let div124_out1 = add131_out1.div((constant1607_out1).unsqueeze_dims(&[0isize]));
        let add132_out1 = squeeze42_out1.clone().add(squeeze44_out1.clone());
        let constant1608_out1 = self.constant1608.val();
        let div125_out1 = add132_out1.div((constant1608_out1).unsqueeze_dims(&[0isize]));
        let sub11_out1 = squeeze43_out1.sub(squeeze41_out1);
        let sub12_out1 = squeeze44_out1.sub(squeeze42_out1);
        let unsqueeze252_out1: Tensor<3> = div124_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze253_out1: Tensor<3> = div125_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze254_out1: Tensor<3> = sub11_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze255_out1: Tensor<3> = sub12_out1.unsqueeze_dims::<3>(&[-1]);
        let concat142_out1 = burn::tensor::Tensor::cat(
            [unsqueeze252_out1, unsqueeze253_out1, unsqueeze254_out1, unsqueeze255_out1]
                .into(),
            2,
        );
        let unsqueeze256_out1: Tensor<4> = concat142_out1.unsqueeze_dims::<4>(&[2]);
        let add133_out1 = mul223_out1.clone().add(clip1_out1.clone());
        let transpose74_out1 = add133_out1.permute([1, 0, 2]);
        let transpose75_out1 = mul223_out1.clone().permute([1, 0, 2]);
        let shape174_out1: [i64; 3] = {
            let axes = &transpose74_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather160_out1 = shape174_out1[0] as i64;
        let gather161_out1 = shape174_out1[1] as i64;
        let gather162_out1 = shape174_out1[2] as i64;
        let constant1617_out1 = 8i64;
        let div126_out1 = gather162_out1 / constant1617_out1;
        let linear89_out1 = self.linear89.forward(transpose74_out1.clone());
        let linear90_out1 = self.linear90.forward(transpose74_out1);
        let linear91_out1 = self.linear91.forward(transpose75_out1);
        let constant1618_out1 = 8i64;
        let mul231_out1 = gather161_out1 * constant1618_out1;
        let unsqueeze257_out1 = [gather160_out1 as i64];
        let unsqueeze258_out1 = [mul231_out1 as i64];
        let unsqueeze259_out1 = [div126_out1 as i64];
        let concat143_out1: [i64; 3usize] = [
            &unsqueeze257_out1[..],
            &unsqueeze258_out1[..],
            &unsqueeze259_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape77_out1 = linear89_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat143_out1));
        let transpose76_out1 = reshape77_out1.permute([1, 0, 2]);
        let shape177_out1: [i64; 3] = {
            let axes = &linear90_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather163_out1 = shape177_out1[0] as i64;
        let unsqueeze260_out1 = [gather163_out1 as i64];
        let unsqueeze261_out1 = [mul231_out1 as i64];
        let unsqueeze262_out1 = [div126_out1 as i64];
        let concat144_out1: [i64; 3usize] = [
            &unsqueeze260_out1[..],
            &unsqueeze261_out1[..],
            &unsqueeze262_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape78_out1 = linear90_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat144_out1));
        let shape178_out1: [i64; 3] = {
            let axes = &linear91_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather164_out1 = shape178_out1[0] as i64;
        let unsqueeze263_out1 = [gather164_out1 as i64];
        let unsqueeze264_out1 = [mul231_out1 as i64];
        let unsqueeze265_out1 = [div126_out1 as i64];
        let concat145_out1: [i64; 3usize] = [
            &unsqueeze263_out1[..],
            &unsqueeze264_out1[..],
            &unsqueeze265_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape79_out1 = linear91_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat145_out1));
        let transpose77_out1 = reshape79_out1.permute([1, 0, 2]);
        let constant1630_out1 = self.constant1630.val();
        let mul232_out1 = transpose76_out1
            .mul((constant1630_out1).unsqueeze_dims(&[0isize, 1isize]));
        let transpose78_out1 = reshape78_out1.permute([1, 2, 0]);
        let matmul121_out1 = mul232_out1.matmul(transpose78_out1);
        let softmax20_out1 = burn::tensor::activation::softmax(matmul121_out1, 2);
        let matmul122_out1 = softmax20_out1.matmul(transpose77_out1);
        let transpose79_out1 = matmul122_out1.permute([1, 0, 2]);
        let mul233_out1 = gather160_out1 * gather161_out1;
        let unsqueeze266_out1 = [mul233_out1 as i64];
        let unsqueeze267_out1 = [gather162_out1 as i64];
        let concat146_out1: [i64; 2usize] = [
            &unsqueeze266_out1[..],
            &unsqueeze267_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape80_out1 = transpose79_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat146_out1));
        let linear92_out1 = self.linear92.forward(reshape80_out1);
        let shape179_out1: [i64; 2] = {
            let axes = &linear92_out1.clone().dims()[0..2];
            let mut output = [0i64; 2];
            for i in 0..2 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather165_out1 = shape179_out1[1] as i64;
        let unsqueeze268_out1 = [gather160_out1 as i64];
        let unsqueeze269_out1 = [gather161_out1 as i64];
        let unsqueeze270_out1 = [gather165_out1 as i64];
        let concat147_out1: [i64; 3usize] = [
            &unsqueeze268_out1[..],
            &unsqueeze269_out1[..],
            &unsqueeze270_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape81_out1 = linear92_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat147_out1));
        let transpose80_out1 = reshape81_out1.permute([1, 0, 2]);
        let add134_out1 = mul223_out1.clone().add(transpose80_out1);
        let constant1637_out1 = self.constant1637.val();
        let pow7_out1 = add134_out1
            .clone()
            .powf((constant1637_out1).unsqueeze_dims(&[0isize, 1isize]));
        let reducemean7_out1 = { pow7_out1.mean_dim(2usize) };
        let constant1638_out1 = self.constant1638.val();
        let add135_out1 = reducemean7_out1
            .add((constant1638_out1).unsqueeze_dims(&[0isize, 1isize]));
        let sqrt43_out1 = add135_out1.sqrt();
        let constant1639_out1 = self.constant1639.val();
        let div127_out1 = (constant1639_out1)
            .unsqueeze_dims(&[0isize, 1isize])
            .div(sqrt43_out1);
        let mul234_out1 = add134_out1.mul(div127_out1);
        let constant124_out1 = self.constant124.val();
        let mul235_out1 = mul234_out1
            .mul((constant124_out1).unsqueeze_dims(&[0isize, 1isize]));
        let add136_out1 = mul235_out1.clone().add(clip1_out1);
        let shape180_out1: [i64; 3] = {
            let axes = &add136_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather166_out1 = shape180_out1[0] as i64;
        let gather167_out1 = shape180_out1[1] as i64;
        let linear93_out1 = self.linear93.forward(add136_out1.clone());
        let unsqueeze271_out1 = [gather166_out1 as i64];
        let unsqueeze272_out1 = [gather167_out1 as i64];
        let constant1646_out1: [i64; 1] = [2i64];
        let constant1644_out1: [i64; 1] = [8i64];
        let constant1645_out1: [i64; 1] = [12i64];
        let concat148_out1: [i64; 5usize] = [
            &unsqueeze271_out1[..],
            &unsqueeze272_out1[..],
            &constant1644_out1[..],
            &constant1645_out1[..],
            &constant1646_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape82_out1 = linear93_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat148_out1));
        let linear94_out1 = self.linear94.forward(add136_out1);
        let unsqueeze273_out1 = [gather166_out1 as i64];
        let unsqueeze274_out1 = [gather167_out1 as i64];
        let constant1650_out1: [i64; 1] = [12i64];
        let constant1649_out1: [i64; 1] = [8i64];
        let concat149_out1: [i64; 4usize] = [
            &unsqueeze273_out1[..],
            &unsqueeze274_out1[..],
            &constant1649_out1[..],
            &constant1650_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape83_out1 = linear94_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat149_out1));
        let softmax21_out1 = burn::tensor::activation::softmax(reshape83_out1, 3);
        let mul236_out1 = reshape82_out1
            .mul((constant339_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let unsqueeze275_out1: Tensor<5> = unsqueeze256_out1.unsqueeze_dims::<5>(&[2]);
        let slice155_out1 = unsqueeze275_out1.clone().slice(s![.., .., .., .., 2..]);
        let mul237_out1 = mul236_out1.mul(slice155_out1);
        let constant1656_out1 = self.constant1656.val();
        let mul238_out1 = mul237_out1
            .mul((constant1656_out1).unsqueeze_dims(&[0isize, 1isize, 2isize, 3isize]));
        let slice156_out1 = unsqueeze275_out1.slice(s![.., .., .., .., 0..2]);
        let add137_out1 = slice156_out1.add(mul238_out1);
        let gather168_out1 = shape148_out1[0] as i64;
        let gather169_out1 = shape148_out1[1] as i64;
        let gather170_out1 = shape148_out1[2] as i64;
        let shape185_out1: [i64; 5] = {
            let axes = &add137_out1.clone().dims()[0..5];
            let mut output = [0i64; 5];
            for i in 0..5 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather171_out1 = shape185_out1[1] as i64;
        let constant1665_out1 = self.constant1665.val();
        let mul239_out1 = add137_out1
            .mul((constant1665_out1).unsqueeze_dims(&[0isize, 1isize, 2isize, 3isize]));
        let constant1666_out1 = self.constant1666.val();
        let sub13_out1 = mul239_out1
            .sub((constant1666_out1).unsqueeze_dims(&[0isize, 1isize, 2isize, 3isize]));
        let transpose81_out1 = sub13_out1.permute([0, 2, 1, 3, 4]);
        let shape186_out1: [i64; 5] = {
            let axes = &transpose81_out1.clone().dims()[0..5];
            let mut output = [0i64; 5];
            for i in 0..5 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice157_out1: [i64; 0] = shape186_out1[0..0].try_into().unwrap();
        let slice158_out1: [i64; 3] = shape186_out1[2..5].try_into().unwrap();
        let constant1673_out1: [i64; 1] = [-1i64];
        let concat150_out1: [i64; 4usize] = [
            &slice157_out1[..],
            &constant1673_out1[..],
            &slice158_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape84_out1 = transpose81_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat150_out1));
        let [split21_out1, split21_out2, split21_out3] = {
            let split_tensors = reshape84_out1.split_with_sizes([3, 6, 3].into(), 2);
            split_tensors.try_into().unwrap()
        };
        let mul240_out1 = gather168_out1 * gather169_out1;
        let unsqueeze276_out1 = [mul240_out1 as i64];
        let unsqueeze277_out1 = [gather170_out1 as i64];
        let unsqueeze278_out1 = [gather107_out1 as i64];
        let unsqueeze279_out1 = [gather108_out1 as i64];
        let concat151_out1: [i64; 4usize] = [
            &unsqueeze276_out1[..],
            &unsqueeze277_out1[..],
            &unsqueeze278_out1[..],
            &unsqueeze279_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape85_out1 = slice136_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat151_out1));
        let gridsample7_out1 = reshape85_out1
            .grid_sample_2d(
                split21_out1,
                burn::tensor::ops::GridSampleOptions::new(
                        burn::tensor::ops::GridSampleMode::Bilinear,
                    )
                    .with_padding_mode(burn::tensor::ops::GridSamplePaddingMode::Zeros)
                    .with_align_corners(false),
            );
        let unsqueeze280_out1 = [mul240_out1 as i64];
        let unsqueeze281_out1 = [gather170_out1 as i64];
        let unsqueeze282_out1 = [gather109_out1 as i64];
        let unsqueeze283_out1 = [gather110_out1 as i64];
        let concat152_out1: [i64; 4usize] = [
            &unsqueeze280_out1[..],
            &unsqueeze281_out1[..],
            &unsqueeze282_out1[..],
            &unsqueeze283_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape86_out1 = slice137_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat152_out1));
        let gridsample8_out1 = reshape86_out1
            .grid_sample_2d(
                split21_out2,
                burn::tensor::ops::GridSampleOptions::new(
                        burn::tensor::ops::GridSampleMode::Bilinear,
                    )
                    .with_padding_mode(burn::tensor::ops::GridSamplePaddingMode::Zeros)
                    .with_align_corners(false),
            );
        let unsqueeze284_out1 = [mul240_out1 as i64];
        let unsqueeze285_out1 = [gather170_out1 as i64];
        let unsqueeze286_out1 = [gather111_out1 as i64];
        let unsqueeze287_out1 = [gather112_out1 as i64];
        let concat153_out1: [i64; 4usize] = [
            &unsqueeze284_out1[..],
            &unsqueeze285_out1[..],
            &unsqueeze286_out1[..],
            &unsqueeze287_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape87_out1 = slice138_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat153_out1));
        let gridsample9_out1 = reshape87_out1
            .grid_sample_2d(
                split21_out3,
                burn::tensor::ops::GridSampleOptions::new(
                        burn::tensor::ops::GridSampleMode::Bilinear,
                    )
                    .with_padding_mode(burn::tensor::ops::GridSamplePaddingMode::Zeros)
                    .with_align_corners(false),
            );
        let transpose82_out1 = softmax21_out1.permute([0, 2, 1, 3]);
        let unsqueeze288_out1 = [mul240_out1 as i64];
        let unsqueeze289_out1 = [gather171_out1 as i64];
        let constant1689_out1: [i64; 1] = [12i64];
        let constant1687_out1: [i64; 1] = [1i64];
        let concat154_out1: [i64; 4usize] = [
            &unsqueeze288_out1[..],
            &constant1687_out1[..],
            &unsqueeze289_out1[..],
            &constant1689_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape88_out1 = transpose82_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat154_out1));
        let concat155_out1 = burn::tensor::Tensor::cat(
            [gridsample7_out1, gridsample8_out1, gridsample9_out1].into(),
            3,
        );
        let mul241_out1 = concat155_out1.mul(reshape88_out1);
        let reducesum3_out1 = {
            mul241_out1.sum_dim(3usize).squeeze_dims::<3usize>(&[3])
        };
        let mul242_out1 = gather169_out1 * gather170_out1;
        let unsqueeze290_out1 = [gather168_out1 as i64];
        let unsqueeze291_out1 = [mul242_out1 as i64];
        let unsqueeze292_out1 = [gather171_out1 as i64];
        let concat156_out1: [i64; 3usize] = [
            &unsqueeze290_out1[..],
            &unsqueeze291_out1[..],
            &unsqueeze292_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape89_out1 = reducesum3_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat156_out1));
        let transpose83_out1 = reshape89_out1.permute([0, 2, 1]);
        let concat157_out1 = burn::tensor::Tensor::cat(
            [mul235_out1.clone(), transpose83_out1.clone()].into(),
            2,
        );
        let linear95_out1 = self.linear95.forward(concat157_out1);
        let sigmoid49_out1 = burn::tensor::activation::sigmoid(linear95_out1);
        let shape187_out1: [i64; 3] = {
            let axes = &sigmoid49_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant1693_out1: [i64; 1] = [-1i64];
        let gather172_out1: [i64; 1usize] = constant1693_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape187_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape187_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant1695_out1: [i64; 1] = [1i64];
        let add138_out1 = {
            let __lhs = gather172_out1;
            let __rhs = constant1695_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant1696_out1: [i64; 1] = [2i64];
        let div128_out1 = {
            let __lhs = add138_out1;
            let __rhs = constant1696_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| if __rhs[__i] != 0 { __lhs[__i] / __rhs[__i] } else { __lhs[__i] })
        };
        let slice159_out1 = sigmoid49_out1.clone().slice(s![.., .., 0..div128_out1[0]]);
        let constant1698_out1: [i64; 1] = [2i64];
        let mul244_out1 = {
            let __lhs = div128_out1;
            let __rhs = constant1698_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice160_out1 = sigmoid49_out1
            .slice(s![.., .., div128_out1[0]..mul244_out1[0]]);
        let mul245_out1 = slice159_out1.mul(mul235_out1);
        let mul246_out1 = slice160_out1.mul(transpose83_out1);
        let add139_out1 = mul245_out1.add(mul246_out1);
        let constant1699_out1 = self.constant1699.val();
        let pow8_out1 = add139_out1
            .clone()
            .powf((constant1699_out1).unsqueeze_dims(&[0isize, 1isize]));
        let reducemean8_out1 = { pow8_out1.mean_dim(2usize) };
        let constant1700_out1 = self.constant1700.val();
        let add140_out1 = reducemean8_out1
            .add((constant1700_out1).unsqueeze_dims(&[0isize, 1isize]));
        let sqrt44_out1 = add140_out1.sqrt();
        let constant1701_out1 = self.constant1701.val();
        let div129_out1 = (constant1701_out1)
            .unsqueeze_dims(&[0isize, 1isize])
            .div(sqrt44_out1);
        let mul247_out1 = add139_out1.mul(div129_out1);
        let constant128_out1 = self.constant128.val();
        let mul248_out1 = mul247_out1
            .mul((constant128_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear96_out1 = self.linear96.forward(mul248_out1.clone());
        let shape188_out1: [i64; 3] = {
            let axes = &linear96_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant1702_out1: [i64; 1] = [-1i64];
        let gather173_out1: [i64; 1usize] = constant1702_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape188_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape188_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant1704_out1: [i64; 1] = [1i64];
        let add141_out1 = {
            let __lhs = gather173_out1;
            let __rhs = constant1704_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant1705_out1: [i64; 1] = [2i64];
        let div130_out1 = {
            let __lhs = add141_out1;
            let __rhs = constant1705_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| if __rhs[__i] != 0 { __lhs[__i] / __rhs[__i] } else { __lhs[__i] })
        };
        let slice161_out1 = linear96_out1.clone().slice(s![.., .., 0..div130_out1[0]]);
        let constant1707_out1: [i64; 1] = [2i64];
        let mul250_out1 = {
            let __lhs = div130_out1;
            let __rhs = constant1707_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice162_out1 = linear96_out1
            .slice(s![.., .., div130_out1[0]..mul250_out1[0]]);
        let sigmoid50_out1 = burn::tensor::activation::sigmoid(slice161_out1.clone());
        let mul251_out1 = slice161_out1.mul(sigmoid50_out1);
        let mul252_out1 = mul251_out1.mul(slice162_out1);
        let linear97_out1 = self.linear97.forward(mul252_out1);
        let add142_out1 = mul248_out1.add(linear97_out1);
        let clip7_out1 = add142_out1.clamp(-65504f64, 65504f64);
        let constant1710_out1 = self.constant1710.val();
        let pow9_out1 = clip7_out1
            .clone()
            .powf((constant1710_out1).unsqueeze_dims(&[0isize, 1isize]));
        let reducemean9_out1 = { pow9_out1.mean_dim(2usize) };
        let constant1711_out1 = self.constant1711.val();
        let add143_out1 = reducemean9_out1
            .add((constant1711_out1).unsqueeze_dims(&[0isize, 1isize]));
        let sqrt45_out1 = add143_out1.sqrt();
        let constant1712_out1 = self.constant1712.val();
        let div131_out1 = (constant1712_out1)
            .unsqueeze_dims(&[0isize, 1isize])
            .div(sqrt45_out1);
        let mul253_out1 = clip7_out1.mul(div131_out1);
        let constant131_out1 = self.constant131.val();
        let mul254_out1 = mul253_out1
            .mul((constant131_out1).unsqueeze_dims(&[0isize, 1isize]));
        let add144_out1 = mul254_out1.clone().add(mul223_out1);
        let linear98_out1 = self.linear98.forward(add144_out1);
        let sigmoid51_out1 = burn::tensor::activation::sigmoid(linear98_out1.clone());
        let mul255_out1 = linear98_out1.mul(sigmoid51_out1);
        let linear99_out1 = self.linear99.forward(mul255_out1);
        let sigmoid52_out1 = burn::tensor::activation::sigmoid(linear99_out1.clone());
        let mul256_out1 = linear99_out1.mul(sigmoid52_out1);
        let linear100_out1 = self.linear100.forward(mul256_out1);
        let add145_out1 = linear100_out1.add(add124_out1);
        (add145_out1, mul254_out1)
    }
}
#[derive(Module, Debug)]
pub struct Submodule23 {
    constant1720: burn::module::Param<Tensor<1>>,
    constant1730: burn::module::Param<Tensor<1>>,
    constant1731: burn::module::Param<Tensor<1>>,
    linear101: Linear,
    linear102: Linear,
    linear103: Linear,
    constant1753: burn::module::Param<Tensor<1>>,
    linear104: Linear,
    constant1760: burn::module::Param<Tensor<1>>,
    constant1761: burn::module::Param<Tensor<1>>,
    constant1762: burn::module::Param<Tensor<1>>,
    constant134: burn::module::Param<Tensor<1>>,
    linear105: Linear,
    linear106: Linear,
    constant1779: burn::module::Param<Tensor<1>>,
    constant1788: burn::module::Param<Tensor<1>>,
    constant1789: burn::module::Param<Tensor<1>>,
    linear107: Linear,
    device: AllocationDevice,
}
impl Submodule23 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let constant1720: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1730: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1731: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear101 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear102 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear103 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let constant1753: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.20412415266036987f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear104 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let constant1760: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1761: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.0000009999999974752427f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1762: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant134: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([192], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [192].into(),
        );
        let linear105 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear106 = LinearConfig::new(192, 96).with_bias(true).init(device);
        let constant1779: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1788: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1789: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear107 = LinearConfig::new(384, 384).with_bias(true).init(device);
        Self {
            constant1720,
            constant1730,
            constant1731,
            linear101,
            linear102,
            linear103,
            constant1753,
            linear104,
            constant1760,
            constant1761,
            constant1762,
            constant134,
            linear105,
            linear106,
            constant1779,
            constant1788,
            constant1789,
            linear107,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `add145_out1`: expected dtype `DType::F32`
    /// - `constant1469_out1`: expected dtype `DType::F32`
    /// - `constant100_out1`: expected dtype `DType::I64`
    /// - `gather134_out1`: expected dtype `DType::F32`
    /// - `gather132_out1`: expected dtype `DType::F32`
    /// - `gather137_out1`: expected dtype `DType::F32`
    /// - `gather135_out1`: expected dtype `DType::F32`
    /// - `mul254_out1`: expected dtype `DType::F32`
    /// - `clip1_out1`: expected dtype `DType::F32`
    /// - `constant339_out1`: expected dtype `DType::F32`
    /// - `slice136_out1`: expected dtype `DType::F32`
    /// - `slice137_out1`: expected dtype `DType::F32`
    /// - `slice138_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        add145_out1: Tensor<3>,
        constant1469_out1: Tensor<1>,
        constant100_out1: Tensor<1, Int>,
        gather134_out1: Tensor<2>,
        gather132_out1: Tensor<2>,
        gather137_out1: Tensor<2>,
        gather135_out1: Tensor<2>,
        mul254_out1: Tensor<3>,
        clip1_out1: Tensor<3>,
        constant339_out1: Tensor<2>,
        shape148_out1: [i64; 4],
        gather107_out1: i64,
        gather108_out1: i64,
        slice136_out1: Tensor<4>,
        gather109_out1: i64,
        gather110_out1: i64,
        slice137_out1: Tensor<4>,
        gather111_out1: i64,
        gather112_out1: i64,
        slice138_out1: Tensor<4>,
    ) -> Tensor<3> {
        let shape189_out1: [i64; 3] = {
            let axes = &add145_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather174_out1 = shape189_out1[0] as i64;
        let gather175_out1 = shape189_out1[1] as i64;
        let reshape90_out1 = add145_out1
            .reshape(burn::tensor::ZeroDimPlaceholders([-1, 33]));
        let softmax22_out1 = burn::tensor::activation::softmax(reshape90_out1, 1);
        let matmul131_out1 = softmax22_out1
            .matmul(constant1469_out1.unsqueeze_dims(&[-1isize]))
            .squeeze_dim::<1usize>(1usize);
        let reshape91_out1 = matmul131_out1
            .reshape(burn::tensor::ZeroDimPlaceholders([-1, 4]));
        let unsqueeze293_out1 = [gather174_out1 as i64];
        let unsqueeze294_out1 = [gather175_out1 as i64];
        let constant1719_out1: [i64; 1] = [-1i64];
        let concat158_out1: [i64; 3usize] = [
            &unsqueeze293_out1[..],
            &unsqueeze294_out1[..],
            &constant1719_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape92_out1 = reshape91_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat158_out1));
        let abs3_out1 = constant100_out1.abs();
        let cast180_out1 = abs3_out1.float().cast(burn::tensor::DType::F32);
        let constant1720_out1 = self.constant1720.val();
        let mul257_out1 = cast180_out1.clone().mul(constant1720_out1);
        let gather176_out1 = {
            let sliced = reshape92_out1.clone().slice(s![.., .., 0]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let add146_out1 = (mul257_out1.clone())
            .unsqueeze_dims(&[0isize])
            .add(gather176_out1);
        let div132_out1 = gather134_out1
            .div((cast180_out1.clone()).unsqueeze_dims(&[0isize]));
        let mul258_out1 = add146_out1.mul(div132_out1.clone());
        let sub14_out1 = gather132_out1.clone().sub(mul258_out1);
        let gather177_out1 = {
            let sliced = reshape92_out1.clone().slice(s![.., .., 1]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let add147_out1 = (mul257_out1.clone())
            .unsqueeze_dims(&[0isize])
            .add(gather177_out1);
        let div133_out1 = gather137_out1.div((cast180_out1).unsqueeze_dims(&[0isize]));
        let mul259_out1 = add147_out1.mul(div133_out1.clone());
        let sub15_out1 = gather135_out1.clone().sub(mul259_out1);
        let gather178_out1 = {
            let sliced = reshape92_out1.clone().slice(s![.., .., 2]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let add148_out1 = (mul257_out1.clone())
            .unsqueeze_dims(&[0isize])
            .add(gather178_out1);
        let mul260_out1 = add148_out1.mul(div132_out1);
        let add149_out1 = gather132_out1.add(mul260_out1);
        let gather179_out1 = {
            let sliced = reshape92_out1.slice(s![.., .., 3]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let add150_out1 = (mul257_out1).unsqueeze_dims(&[0isize]).add(gather179_out1);
        let mul261_out1 = add150_out1.mul(div133_out1);
        let add151_out1 = gather135_out1.add(mul261_out1);
        let unsqueeze295_out1: Tensor<3> = sub14_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze296_out1: Tensor<3> = sub15_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze297_out1: Tensor<3> = add149_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze298_out1: Tensor<3> = add151_out1.unsqueeze_dims::<3>(&[-1]);
        let concat159_out1 = burn::tensor::Tensor::cat(
            [unsqueeze295_out1, unsqueeze296_out1, unsqueeze297_out1, unsqueeze298_out1]
                .into(),
            2,
        );
        let [split22_out1, split22_out2, split22_out3, split22_out4] = {
            let split_tensors = concat159_out1.split_with_sizes([1, 1, 1, 1].into(), 2);
            split_tensors.try_into().unwrap()
        };
        let squeeze45_out1 = split22_out1.squeeze_dims::<2>(&[-1]);
        let squeeze46_out1 = split22_out2.squeeze_dims::<2>(&[-1]);
        let squeeze47_out1 = split22_out3.squeeze_dims::<2>(&[-1]);
        let squeeze48_out1 = split22_out4.squeeze_dims::<2>(&[-1]);
        let add152_out1 = squeeze45_out1.clone().add(squeeze47_out1.clone());
        let constant1730_out1 = self.constant1730.val();
        let div134_out1 = add152_out1.div((constant1730_out1).unsqueeze_dims(&[0isize]));
        let add153_out1 = squeeze46_out1.clone().add(squeeze48_out1.clone());
        let constant1731_out1 = self.constant1731.val();
        let div135_out1 = add153_out1.div((constant1731_out1).unsqueeze_dims(&[0isize]));
        let sub16_out1 = squeeze47_out1.sub(squeeze45_out1);
        let sub17_out1 = squeeze48_out1.sub(squeeze46_out1);
        let unsqueeze299_out1: Tensor<3> = div134_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze300_out1: Tensor<3> = div135_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze301_out1: Tensor<3> = sub16_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze302_out1: Tensor<3> = sub17_out1.unsqueeze_dims::<3>(&[-1]);
        let concat160_out1 = burn::tensor::Tensor::cat(
            [unsqueeze299_out1, unsqueeze300_out1, unsqueeze301_out1, unsqueeze302_out1]
                .into(),
            2,
        );
        let unsqueeze303_out1: Tensor<4> = concat160_out1.unsqueeze_dims::<4>(&[2]);
        let add154_out1 = mul254_out1.clone().add(clip1_out1.clone());
        let transpose84_out1 = add154_out1.permute([1, 0, 2]);
        let transpose85_out1 = mul254_out1.clone().permute([1, 0, 2]);
        let shape191_out1: [i64; 3] = {
            let axes = &transpose84_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather180_out1 = shape191_out1[0] as i64;
        let gather181_out1 = shape191_out1[1] as i64;
        let gather182_out1 = shape191_out1[2] as i64;
        let constant1740_out1 = 8i64;
        let div136_out1 = gather182_out1 / constant1740_out1;
        let linear101_out1 = self.linear101.forward(transpose84_out1.clone());
        let linear102_out1 = self.linear102.forward(transpose84_out1);
        let linear103_out1 = self.linear103.forward(transpose85_out1);
        let constant1741_out1 = 8i64;
        let mul262_out1 = gather181_out1 * constant1741_out1;
        let unsqueeze304_out1 = [gather180_out1 as i64];
        let unsqueeze305_out1 = [mul262_out1 as i64];
        let unsqueeze306_out1 = [div136_out1 as i64];
        let concat161_out1: [i64; 3usize] = [
            &unsqueeze304_out1[..],
            &unsqueeze305_out1[..],
            &unsqueeze306_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape93_out1 = linear101_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat161_out1));
        let transpose86_out1 = reshape93_out1.permute([1, 0, 2]);
        let shape194_out1: [i64; 3] = {
            let axes = &linear102_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather183_out1 = shape194_out1[0] as i64;
        let unsqueeze307_out1 = [gather183_out1 as i64];
        let unsqueeze308_out1 = [mul262_out1 as i64];
        let unsqueeze309_out1 = [div136_out1 as i64];
        let concat162_out1: [i64; 3usize] = [
            &unsqueeze307_out1[..],
            &unsqueeze308_out1[..],
            &unsqueeze309_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape94_out1 = linear102_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat162_out1));
        let shape195_out1: [i64; 3] = {
            let axes = &linear103_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather184_out1 = shape195_out1[0] as i64;
        let unsqueeze310_out1 = [gather184_out1 as i64];
        let unsqueeze311_out1 = [mul262_out1 as i64];
        let unsqueeze312_out1 = [div136_out1 as i64];
        let concat163_out1: [i64; 3usize] = [
            &unsqueeze310_out1[..],
            &unsqueeze311_out1[..],
            &unsqueeze312_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape95_out1 = linear103_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat163_out1));
        let transpose87_out1 = reshape95_out1.permute([1, 0, 2]);
        let constant1753_out1 = self.constant1753.val();
        let mul263_out1 = transpose86_out1
            .mul((constant1753_out1).unsqueeze_dims(&[0isize, 1isize]));
        let transpose88_out1 = reshape94_out1.permute([1, 2, 0]);
        let matmul135_out1 = mul263_out1.matmul(transpose88_out1);
        let softmax23_out1 = burn::tensor::activation::softmax(matmul135_out1, 2);
        let matmul136_out1 = softmax23_out1.matmul(transpose87_out1);
        let transpose89_out1 = matmul136_out1.permute([1, 0, 2]);
        let mul264_out1 = gather180_out1 * gather181_out1;
        let unsqueeze313_out1 = [mul264_out1 as i64];
        let unsqueeze314_out1 = [gather182_out1 as i64];
        let concat164_out1: [i64; 2usize] = [
            &unsqueeze313_out1[..],
            &unsqueeze314_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape96_out1 = transpose89_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat164_out1));
        let linear104_out1 = self.linear104.forward(reshape96_out1);
        let shape196_out1: [i64; 2] = {
            let axes = &linear104_out1.clone().dims()[0..2];
            let mut output = [0i64; 2];
            for i in 0..2 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather185_out1 = shape196_out1[1] as i64;
        let unsqueeze315_out1 = [gather180_out1 as i64];
        let unsqueeze316_out1 = [gather181_out1 as i64];
        let unsqueeze317_out1 = [gather185_out1 as i64];
        let concat165_out1: [i64; 3usize] = [
            &unsqueeze315_out1[..],
            &unsqueeze316_out1[..],
            &unsqueeze317_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape97_out1 = linear104_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat165_out1));
        let transpose90_out1 = reshape97_out1.permute([1, 0, 2]);
        let add155_out1 = mul254_out1.add(transpose90_out1);
        let constant1760_out1 = self.constant1760.val();
        let pow10_out1 = add155_out1
            .clone()
            .powf((constant1760_out1).unsqueeze_dims(&[0isize, 1isize]));
        let reducemean10_out1 = { pow10_out1.mean_dim(2usize) };
        let constant1761_out1 = self.constant1761.val();
        let add156_out1 = reducemean10_out1
            .add((constant1761_out1).unsqueeze_dims(&[0isize, 1isize]));
        let sqrt46_out1 = add156_out1.sqrt();
        let constant1762_out1 = self.constant1762.val();
        let div137_out1 = (constant1762_out1)
            .unsqueeze_dims(&[0isize, 1isize])
            .div(sqrt46_out1);
        let mul265_out1 = add155_out1.mul(div137_out1);
        let constant134_out1 = self.constant134.val();
        let mul266_out1 = mul265_out1
            .mul((constant134_out1).unsqueeze_dims(&[0isize, 1isize]));
        let add157_out1 = mul266_out1.clone().add(clip1_out1);
        let shape197_out1: [i64; 3] = {
            let axes = &add157_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather186_out1 = shape197_out1[0] as i64;
        let gather187_out1 = shape197_out1[1] as i64;
        let linear105_out1 = self.linear105.forward(add157_out1.clone());
        let unsqueeze318_out1 = [gather186_out1 as i64];
        let unsqueeze319_out1 = [gather187_out1 as i64];
        let constant1769_out1: [i64; 1] = [2i64];
        let constant1767_out1: [i64; 1] = [8i64];
        let constant1768_out1: [i64; 1] = [12i64];
        let concat166_out1: [i64; 5usize] = [
            &unsqueeze318_out1[..],
            &unsqueeze319_out1[..],
            &constant1767_out1[..],
            &constant1768_out1[..],
            &constant1769_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape98_out1 = linear105_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat166_out1));
        let linear106_out1 = self.linear106.forward(add157_out1);
        let unsqueeze320_out1 = [gather186_out1 as i64];
        let unsqueeze321_out1 = [gather187_out1 as i64];
        let constant1773_out1: [i64; 1] = [12i64];
        let constant1772_out1: [i64; 1] = [8i64];
        let concat167_out1: [i64; 4usize] = [
            &unsqueeze320_out1[..],
            &unsqueeze321_out1[..],
            &constant1772_out1[..],
            &constant1773_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape99_out1 = linear106_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat167_out1));
        let softmax24_out1 = burn::tensor::activation::softmax(reshape99_out1, 3);
        let mul267_out1 = reshape98_out1
            .mul((constant339_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let unsqueeze322_out1: Tensor<5> = unsqueeze303_out1.unsqueeze_dims::<5>(&[2]);
        let slice163_out1 = unsqueeze322_out1.clone().slice(s![.., .., .., .., 2..]);
        let mul268_out1 = mul267_out1.mul(slice163_out1);
        let constant1779_out1 = self.constant1779.val();
        let mul269_out1 = mul268_out1
            .mul((constant1779_out1).unsqueeze_dims(&[0isize, 1isize, 2isize, 3isize]));
        let slice164_out1 = unsqueeze322_out1.slice(s![.., .., .., .., 0..2]);
        let add158_out1 = slice164_out1.add(mul269_out1);
        let gather188_out1 = shape148_out1[0] as i64;
        let gather189_out1 = shape148_out1[1] as i64;
        let gather190_out1 = shape148_out1[2] as i64;
        let shape202_out1: [i64; 5] = {
            let axes = &add158_out1.clone().dims()[0..5];
            let mut output = [0i64; 5];
            for i in 0..5 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather191_out1 = shape202_out1[1] as i64;
        let constant1788_out1 = self.constant1788.val();
        let mul270_out1 = add158_out1
            .mul((constant1788_out1).unsqueeze_dims(&[0isize, 1isize, 2isize, 3isize]));
        let constant1789_out1 = self.constant1789.val();
        let sub18_out1 = mul270_out1
            .sub((constant1789_out1).unsqueeze_dims(&[0isize, 1isize, 2isize, 3isize]));
        let transpose91_out1 = sub18_out1.permute([0, 2, 1, 3, 4]);
        let shape203_out1: [i64; 5] = {
            let axes = &transpose91_out1.clone().dims()[0..5];
            let mut output = [0i64; 5];
            for i in 0..5 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice165_out1: [i64; 0] = shape203_out1[0..0].try_into().unwrap();
        let slice166_out1: [i64; 3] = shape203_out1[2..5].try_into().unwrap();
        let constant1796_out1: [i64; 1] = [-1i64];
        let concat168_out1: [i64; 4usize] = [
            &slice165_out1[..],
            &constant1796_out1[..],
            &slice166_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape100_out1 = transpose91_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat168_out1));
        let [split23_out1, split23_out2, split23_out3] = {
            let split_tensors = reshape100_out1.split_with_sizes([3, 6, 3].into(), 2);
            split_tensors.try_into().unwrap()
        };
        let mul271_out1 = gather188_out1 * gather189_out1;
        let unsqueeze323_out1 = [mul271_out1 as i64];
        let unsqueeze324_out1 = [gather190_out1 as i64];
        let unsqueeze325_out1 = [gather107_out1 as i64];
        let unsqueeze326_out1 = [gather108_out1 as i64];
        let concat169_out1: [i64; 4usize] = [
            &unsqueeze323_out1[..],
            &unsqueeze324_out1[..],
            &unsqueeze325_out1[..],
            &unsqueeze326_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape101_out1 = slice136_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat169_out1));
        let gridsample10_out1 = reshape101_out1
            .grid_sample_2d(
                split23_out1,
                burn::tensor::ops::GridSampleOptions::new(
                        burn::tensor::ops::GridSampleMode::Bilinear,
                    )
                    .with_padding_mode(burn::tensor::ops::GridSamplePaddingMode::Zeros)
                    .with_align_corners(false),
            );
        let unsqueeze327_out1 = [mul271_out1 as i64];
        let unsqueeze328_out1 = [gather190_out1 as i64];
        let unsqueeze329_out1 = [gather109_out1 as i64];
        let unsqueeze330_out1 = [gather110_out1 as i64];
        let concat170_out1: [i64; 4usize] = [
            &unsqueeze327_out1[..],
            &unsqueeze328_out1[..],
            &unsqueeze329_out1[..],
            &unsqueeze330_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape102_out1 = slice137_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat170_out1));
        let gridsample11_out1 = reshape102_out1
            .grid_sample_2d(
                split23_out2,
                burn::tensor::ops::GridSampleOptions::new(
                        burn::tensor::ops::GridSampleMode::Bilinear,
                    )
                    .with_padding_mode(burn::tensor::ops::GridSamplePaddingMode::Zeros)
                    .with_align_corners(false),
            );
        let unsqueeze331_out1 = [mul271_out1 as i64];
        let unsqueeze332_out1 = [gather190_out1 as i64];
        let unsqueeze333_out1 = [gather111_out1 as i64];
        let unsqueeze334_out1 = [gather112_out1 as i64];
        let concat171_out1: [i64; 4usize] = [
            &unsqueeze331_out1[..],
            &unsqueeze332_out1[..],
            &unsqueeze333_out1[..],
            &unsqueeze334_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape103_out1 = slice138_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat171_out1));
        let gridsample12_out1 = reshape103_out1
            .grid_sample_2d(
                split23_out3,
                burn::tensor::ops::GridSampleOptions::new(
                        burn::tensor::ops::GridSampleMode::Bilinear,
                    )
                    .with_padding_mode(burn::tensor::ops::GridSamplePaddingMode::Zeros)
                    .with_align_corners(false),
            );
        let transpose92_out1 = softmax24_out1.permute([0, 2, 1, 3]);
        let unsqueeze335_out1 = [mul271_out1 as i64];
        let unsqueeze336_out1 = [gather191_out1 as i64];
        let constant1812_out1: [i64; 1] = [12i64];
        let constant1810_out1: [i64; 1] = [1i64];
        let concat172_out1: [i64; 4usize] = [
            &unsqueeze335_out1[..],
            &constant1810_out1[..],
            &unsqueeze336_out1[..],
            &constant1812_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape104_out1 = transpose92_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat172_out1));
        let concat173_out1 = burn::tensor::Tensor::cat(
            [gridsample10_out1, gridsample11_out1, gridsample12_out1].into(),
            3,
        );
        let mul272_out1 = concat173_out1.mul(reshape104_out1);
        let reducesum4_out1 = {
            mul272_out1.sum_dim(3usize).squeeze_dims::<3usize>(&[3])
        };
        let mul273_out1 = gather189_out1 * gather190_out1;
        let unsqueeze337_out1 = [gather188_out1 as i64];
        let unsqueeze338_out1 = [mul273_out1 as i64];
        let unsqueeze339_out1 = [gather191_out1 as i64];
        let concat174_out1: [i64; 3usize] = [
            &unsqueeze337_out1[..],
            &unsqueeze338_out1[..],
            &unsqueeze339_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape105_out1 = reducesum4_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat174_out1));
        let transpose93_out1 = reshape105_out1.permute([0, 2, 1]);
        let concat175_out1 = burn::tensor::Tensor::cat(
            [mul266_out1.clone(), transpose93_out1.clone()].into(),
            2,
        );
        let linear107_out1 = self.linear107.forward(concat175_out1);
        let sigmoid53_out1 = burn::tensor::activation::sigmoid(linear107_out1);
        let shape204_out1: [i64; 3] = {
            let axes = &sigmoid53_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant1816_out1: [i64; 1] = [-1i64];
        let gather192_out1: [i64; 1usize] = constant1816_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape204_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape204_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant1818_out1: [i64; 1] = [1i64];
        let add159_out1 = {
            let __lhs = gather192_out1;
            let __rhs = constant1818_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant1819_out1: [i64; 1] = [2i64];
        let div138_out1 = {
            let __lhs = add159_out1;
            let __rhs = constant1819_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| if __rhs[__i] != 0 { __lhs[__i] / __rhs[__i] } else { __lhs[__i] })
        };
        let slice167_out1 = sigmoid53_out1.clone().slice(s![.., .., 0..div138_out1[0]]);
        let constant1821_out1: [i64; 1] = [2i64];
        let mul275_out1 = {
            let __lhs = div138_out1;
            let __rhs = constant1821_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice168_out1 = sigmoid53_out1
            .slice(s![.., .., div138_out1[0]..mul275_out1[0]]);
        let mul276_out1 = slice167_out1.mul(mul266_out1);
        let mul277_out1 = slice168_out1.mul(transpose93_out1);
        let add160_out1 = mul276_out1.add(mul277_out1);
        add160_out1
    }
}
#[derive(Module, Debug)]
pub struct Submodule24 {
    constant1822: burn::module::Param<Tensor<1>>,
    constant1823: burn::module::Param<Tensor<1>>,
    constant1824: burn::module::Param<Tensor<1>>,
    constant138: burn::module::Param<Tensor<1>>,
    linear108: Linear,
    linear109: Linear,
    constant1833: burn::module::Param<Tensor<1>>,
    constant1834: burn::module::Param<Tensor<1>>,
    constant1835: burn::module::Param<Tensor<1>>,
    constant141: burn::module::Param<Tensor<1>>,
    linear110: Linear,
    linear111: Linear,
    linear112: Linear,
    constant1846: burn::module::Param<Tensor<1>>,
    constant1856: burn::module::Param<Tensor<1>>,
    constant1857: burn::module::Param<Tensor<1>>,
    linear113: Linear,
    device: AllocationDevice,
}
impl Submodule24 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let constant1822: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1823: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.0000009999999974752427f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1824: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant138: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([192], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [192].into(),
        );
        let linear108 = LinearConfig::new(192, 512).with_bias(true).init(device);
        let linear109 = LinearConfig::new(256, 192).with_bias(true).init(device);
        let constant1833: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1834: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.0000009999999974752427f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1835: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([1f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant141: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([192], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [192].into(),
        );
        let linear110 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear111 = LinearConfig::new(192, 192).with_bias(true).init(device);
        let linear112 = LinearConfig::new(192, 132).with_bias(true).init(device);
        let constant1846: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1856: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1857: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([2f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let linear113 = LinearConfig::new(192, 18).with_bias(true).init(device);
        Self {
            constant1822,
            constant1823,
            constant1824,
            constant138,
            linear108,
            linear109,
            constant1833,
            constant1834,
            constant1835,
            constant141,
            linear110,
            linear111,
            linear112,
            constant1846,
            constant1856,
            constant1857,
            linear113,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `add160_out1`: expected dtype `DType::F32`
    /// - `mul254_out1`: expected dtype `DType::F32`
    /// - `add145_out1`: expected dtype `DType::F32`
    /// - `constant1469_out1`: expected dtype `DType::F32`
    /// - `constant100_out1`: expected dtype `DType::I64`
    /// - `gather134_out1`: expected dtype `DType::F32`
    /// - `gather132_out1`: expected dtype `DType::F32`
    /// - `gather137_out1`: expected dtype `DType::F32`
    /// - `gather135_out1`: expected dtype `DType::F32`
    pub fn forward(
        &self,
        add160_out1: Tensor<3>,
        mul254_out1: Tensor<3>,
        add145_out1: Tensor<3>,
        constant1469_out1: Tensor<1>,
        constant100_out1: Tensor<1, Int>,
        gather134_out1: Tensor<2>,
        gather132_out1: Tensor<2>,
        gather137_out1: Tensor<2>,
        gather135_out1: Tensor<2>,
    ) -> (Tensor<3>, Tensor<3>, Tensor<3>, Tensor<3>) {
        let constant1822_out1 = self.constant1822.val();
        let pow11_out1 = add160_out1
            .clone()
            .powf((constant1822_out1).unsqueeze_dims(&[0isize, 1isize]));
        let reducemean11_out1 = { pow11_out1.mean_dim(2usize) };
        let constant1823_out1 = self.constant1823.val();
        let add161_out1 = reducemean11_out1
            .add((constant1823_out1).unsqueeze_dims(&[0isize, 1isize]));
        let sqrt47_out1 = add161_out1.sqrt();
        let constant1824_out1 = self.constant1824.val();
        let div139_out1 = (constant1824_out1)
            .unsqueeze_dims(&[0isize, 1isize])
            .div(sqrt47_out1);
        let mul278_out1 = add160_out1.mul(div139_out1);
        let constant138_out1 = self.constant138.val();
        let mul279_out1 = mul278_out1
            .mul((constant138_out1).unsqueeze_dims(&[0isize, 1isize]));
        let linear108_out1 = self.linear108.forward(mul279_out1.clone());
        let shape205_out1: [i64; 3] = {
            let axes = &linear108_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant1825_out1: [i64; 1] = [-1i64];
        let gather193_out1: [i64; 1usize] = constant1825_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape205_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape205_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant1827_out1: [i64; 1] = [1i64];
        let add162_out1 = {
            let __lhs = gather193_out1;
            let __rhs = constant1827_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant1828_out1: [i64; 1] = [2i64];
        let div140_out1 = {
            let __lhs = add162_out1;
            let __rhs = constant1828_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| if __rhs[__i] != 0 { __lhs[__i] / __rhs[__i] } else { __lhs[__i] })
        };
        let slice169_out1 = linear108_out1.clone().slice(s![.., .., 0..div140_out1[0]]);
        let constant1830_out1: [i64; 1] = [2i64];
        let mul281_out1 = {
            let __lhs = div140_out1;
            let __rhs = constant1830_out1;
            core::array::from_fn::<
                i64,
                1usize,
                _,
            >(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice170_out1 = linear108_out1
            .slice(s![.., .., div140_out1[0]..mul281_out1[0]]);
        let sigmoid54_out1 = burn::tensor::activation::sigmoid(slice169_out1.clone());
        let mul282_out1 = slice169_out1.mul(sigmoid54_out1);
        let mul283_out1 = mul282_out1.mul(slice170_out1);
        let linear109_out1 = self.linear109.forward(mul283_out1);
        let add163_out1 = mul279_out1.add(linear109_out1);
        let clip8_out1 = add163_out1.clamp(-65504f64, 65504f64);
        let constant1833_out1 = self.constant1833.val();
        let pow12_out1 = clip8_out1
            .clone()
            .powf((constant1833_out1).unsqueeze_dims(&[0isize, 1isize]));
        let reducemean12_out1 = { pow12_out1.mean_dim(2usize) };
        let constant1834_out1 = self.constant1834.val();
        let add164_out1 = reducemean12_out1
            .add((constant1834_out1).unsqueeze_dims(&[0isize, 1isize]));
        let sqrt48_out1 = add164_out1.sqrt();
        let constant1835_out1 = self.constant1835.val();
        let div141_out1 = (constant1835_out1)
            .unsqueeze_dims(&[0isize, 1isize])
            .div(sqrt48_out1);
        let mul284_out1 = clip8_out1.mul(div141_out1);
        let constant141_out1 = self.constant141.val();
        let mul285_out1 = mul284_out1
            .mul((constant141_out1).unsqueeze_dims(&[0isize, 1isize]));
        let add165_out1 = mul285_out1.clone().add(mul254_out1);
        let linear110_out1 = self.linear110.forward(add165_out1);
        let sigmoid55_out1 = burn::tensor::activation::sigmoid(linear110_out1.clone());
        let mul286_out1 = linear110_out1.mul(sigmoid55_out1);
        let linear111_out1 = self.linear111.forward(mul286_out1);
        let sigmoid56_out1 = burn::tensor::activation::sigmoid(linear111_out1.clone());
        let mul287_out1 = linear111_out1.mul(sigmoid56_out1);
        let linear112_out1 = self.linear112.forward(mul287_out1);
        let add166_out1 = linear112_out1.add(add145_out1);
        let shape206_out1: [i64; 3] = {
            let axes = &add166_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather194_out1 = shape206_out1[0] as i64;
        let gather195_out1 = shape206_out1[1] as i64;
        let reshape106_out1 = add166_out1
            .clone()
            .reshape(burn::tensor::ZeroDimPlaceholders([-1, 33]));
        let softmax25_out1 = burn::tensor::activation::softmax(reshape106_out1, 1);
        let matmul145_out1 = softmax25_out1
            .matmul(constant1469_out1.unsqueeze_dims(&[-1isize]))
            .squeeze_dim::<1usize>(1usize);
        let reshape107_out1 = matmul145_out1
            .reshape(burn::tensor::ZeroDimPlaceholders([-1, 4]));
        let unsqueeze340_out1 = [gather194_out1 as i64];
        let unsqueeze341_out1 = [gather195_out1 as i64];
        let constant1842_out1: [i64; 1] = [-1i64];
        let concat176_out1: [i64; 3usize] = [
            &unsqueeze340_out1[..],
            &unsqueeze341_out1[..],
            &constant1842_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let unsqueeze342_out1 = [gather194_out1 as i64];
        let unsqueeze343_out1 = [gather195_out1 as i64];
        let constant1845_out1: [i64; 1] = [-1i64];
        let concat177_out1: [i64; 3usize] = [
            &unsqueeze342_out1[..],
            &unsqueeze343_out1[..],
            &constant1845_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape108_out1 = reshape107_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat176_out1));
        let abs4_out1 = constant100_out1.abs();
        let cast188_out1 = abs4_out1.float().cast(burn::tensor::DType::F32);
        let constant1846_out1 = self.constant1846.val();
        let mul288_out1 = cast188_out1.clone().mul(constant1846_out1);
        let gather196_out1 = {
            let sliced = reshape108_out1.clone().slice(s![.., .., 0]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let add167_out1 = (mul288_out1.clone())
            .unsqueeze_dims(&[0isize])
            .add(gather196_out1);
        let div142_out1 = gather134_out1
            .div((cast188_out1.clone()).unsqueeze_dims(&[0isize]));
        let mul289_out1 = add167_out1.mul(div142_out1.clone());
        let sub19_out1 = gather132_out1.clone().sub(mul289_out1);
        let gather197_out1 = {
            let sliced = reshape108_out1.clone().slice(s![.., .., 1]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let add168_out1 = (mul288_out1.clone())
            .unsqueeze_dims(&[0isize])
            .add(gather197_out1);
        let div143_out1 = gather137_out1.div((cast188_out1).unsqueeze_dims(&[0isize]));
        let mul290_out1 = add168_out1.mul(div143_out1.clone());
        let sub20_out1 = gather135_out1.clone().sub(mul290_out1);
        let gather198_out1 = {
            let sliced = reshape108_out1.clone().slice(s![.., .., 2]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let add169_out1 = (mul288_out1.clone())
            .unsqueeze_dims(&[0isize])
            .add(gather198_out1);
        let mul291_out1 = add169_out1.mul(div142_out1);
        let add170_out1 = gather132_out1.add(mul291_out1);
        let gather199_out1 = {
            let sliced = reshape108_out1.slice(s![.., .., 3]);
            sliced.squeeze_dim::<2usize>(2)
        };
        let add171_out1 = (mul288_out1).unsqueeze_dims(&[0isize]).add(gather199_out1);
        let mul292_out1 = add171_out1.mul(div143_out1);
        let add172_out1 = gather135_out1.add(mul292_out1);
        let unsqueeze344_out1: Tensor<3> = sub19_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze345_out1: Tensor<3> = sub20_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze346_out1: Tensor<3> = add170_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze347_out1: Tensor<3> = add172_out1.unsqueeze_dims::<3>(&[-1]);
        let concat178_out1 = burn::tensor::Tensor::cat(
            [unsqueeze344_out1, unsqueeze345_out1, unsqueeze346_out1, unsqueeze347_out1]
                .into(),
            2,
        );
        let [split24_out1, split24_out2, split24_out3, split24_out4] = {
            let split_tensors = concat178_out1.split_with_sizes([1, 1, 1, 1].into(), 2);
            split_tensors.try_into().unwrap()
        };
        let squeeze49_out1 = split24_out1.squeeze_dims::<2>(&[-1]);
        let squeeze50_out1 = split24_out2.squeeze_dims::<2>(&[-1]);
        let squeeze51_out1 = split24_out3.squeeze_dims::<2>(&[-1]);
        let squeeze52_out1 = split24_out4.squeeze_dims::<2>(&[-1]);
        let add173_out1 = squeeze49_out1.clone().add(squeeze51_out1.clone());
        let constant1856_out1 = self.constant1856.val();
        let div144_out1 = add173_out1.div((constant1856_out1).unsqueeze_dims(&[0isize]));
        let add174_out1 = squeeze50_out1.clone().add(squeeze52_out1.clone());
        let constant1857_out1 = self.constant1857.val();
        let div145_out1 = add174_out1.div((constant1857_out1).unsqueeze_dims(&[0isize]));
        let sub21_out1 = squeeze51_out1.sub(squeeze49_out1);
        let sub22_out1 = squeeze52_out1.sub(squeeze50_out1);
        let unsqueeze348_out1: Tensor<3> = div144_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze349_out1: Tensor<3> = div145_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze350_out1: Tensor<3> = sub21_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze351_out1: Tensor<3> = sub22_out1.unsqueeze_dims::<3>(&[-1]);
        let concat179_out1 = burn::tensor::Tensor::cat(
            [unsqueeze348_out1, unsqueeze349_out1, unsqueeze350_out1, unsqueeze351_out1]
                .into(),
            2,
        );
        let linear113_out1 = self.linear113.forward(mul285_out1.clone());
        let unsqueeze352_out1 = [gather194_out1 as i64];
        let unsqueeze353_out1 = [gather195_out1 as i64];
        let constant1865_out1: [i64; 1] = [33i64];
        let constant1864_out1: [i64; 1] = [4i64];
        let concat180_out1: [i64; 4usize] = [
            &unsqueeze352_out1[..],
            &unsqueeze353_out1[..],
            &constant1864_out1[..],
            &constant1865_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let reshape109_out1 = add166_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat180_out1));
        let softmax26_out1 = burn::tensor::activation::softmax(reshape109_out1, 3);
        let (topk2_out1, topk2_out2) = {
            let (values, indices) = softmax26_out1.topk_with_indices(4, 3);
            (values, indices.cast(burn::tensor::DType::I64))
        };
        let reducemean13_out1 = { topk2_out1.clone().mean_dim(3usize) };
        let concat181_out1 = burn::tensor::Tensor::cat(
            [topk2_out1, reducemean13_out1].into(),
            3,
        );
        let reshape110_out1 = concat181_out1
            .reshape(burn::tensor::ZeroDimPlaceholders(concat177_out1));
        (reshape110_out1, linear113_out1, mul285_out1, concat179_out1)
    }
}
#[derive(Module, Debug)]
pub struct Submodule25 {
    linear114: Linear,
    linear115: Linear,
    linear116: Linear,
    constant392: burn::module::Param<Tensor<2>>,
    constant142: burn::module::Param<Tensor<1>>,
    constant393: burn::module::Param<Tensor<2>>,
    constant143: burn::module::Param<Tensor<1>>,
    constant1875: burn::module::Param<Tensor<1>>,
    constant1876: burn::module::Param<Tensor<1>>,
    constant1877: burn::module::Param<Tensor<1>>,
    constant1878: burn::module::Param<Tensor<1>>,
    constant1886: burn::module::Param<Tensor<1, Int>>,
    constant1887: burn::module::Param<Tensor<1, Int>>,
    device: AllocationDevice,
}
impl Submodule25 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let linear114 = LinearConfig::new(20, 64).with_bias(true).init(device);
        let linear115 = LinearConfig::new(64, 1).with_bias(true).init(device);
        let linear116 = LinearConfig::new(192, 4).with_bias(true).init(device);
        let constant392: burn::module::Param<Tensor<2>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                2,
            >::zeros([20, 64], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [20, 64].into(),
        );
        let constant142: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([64], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [64].into(),
        );
        let constant393: burn::module::Param<Tensor<2>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                2,
            >::zeros([64, 1], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [64, 1].into(),
        );
        let constant143: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::zeros([1], (device, burn::tensor::DType::F32)),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1875: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1876: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1877: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1878: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
            >::from_data(
                burn::tensor::TensorData::from([0.5f64]),
                (device, burn::tensor::DType::F32),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1886: burn::module::Param<Tensor<1, Int>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
                Int,
            >::from_data(
                burn::tensor::TensorData::from([18i64]),
                (device, burn::tensor::DType::I64),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        let constant1887: burn::module::Param<Tensor<1, Int>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| Tensor::<
                1,
                Int,
            >::from_data(
                burn::tensor::TensorData::from([18i64]),
                (device, burn::tensor::DType::I64),
            ),
            device.clone(),
            false,
            [1].into(),
        );
        Self {
            linear114,
            linear115,
            linear116,
            constant392,
            constant142,
            constant393,
            constant143,
            constant1875,
            constant1876,
            constant1877,
            constant1878,
            constant1886,
            constant1887,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `reshape110_out1`: expected dtype `DType::F32`
    /// - `linear113_out1`: expected dtype `DType::F32`
    /// - `mul285_out1`: expected dtype `DType::F32`
    /// - `concat179_out1`: expected dtype `DType::F32`
    /// - `orig_target_sizes`: expected dtype `DType::I64`
    pub fn forward(
        &self,
        reshape110_out1: Tensor<3>,
        linear113_out1: Tensor<3>,
        mul285_out1: Tensor<3>,
        concat179_out1: Tensor<3>,
        orig_target_sizes: Tensor<2, Int>,
    ) -> (Tensor<2, Int>, Tensor<3>, Tensor<2>, Tensor<2, Int>) {
        let linear114_out1 = self.linear114.forward(reshape110_out1.clone());
        let sigmoid57_out1 = burn::tensor::activation::sigmoid(linear114_out1.clone());
        let mul293_out1 = linear114_out1.mul(sigmoid57_out1);
        let linear115_out1 = self.linear115.forward(mul293_out1);
        let add175_out1 = linear113_out1.add(linear115_out1);
        let linear116_out1 = self.linear116.forward(mul285_out1);
        let constant392_out1 = self.constant392.val();
        let matmul150_out1 = reshape110_out1
            .matmul(constant392_out1.unsqueeze::<3usize>());
        let constant142_out1 = self.constant142.val();
        let add176_out1 = (constant142_out1)
            .unsqueeze_dims(&[0isize, 1isize])
            .add(matmul150_out1);
        let sigmoid58_out1 = burn::tensor::activation::sigmoid(add176_out1.clone());
        let mul294_out1 = add176_out1.mul(sigmoid58_out1);
        let constant393_out1 = self.constant393.val();
        let matmul151_out1 = mul294_out1.matmul(constant393_out1.unsqueeze::<3usize>());
        let constant143_out1 = self.constant143.val();
        let add177_out1 = (constant143_out1)
            .unsqueeze_dims(&[0isize, 1isize])
            .add(matmul151_out1);
        let add178_out1 = linear116_out1.add(add177_out1);
        let unsqueeze354_out1: Tensor<4> = concat179_out1.unsqueeze_dims::<4>(&[0]);
        let unsqueeze355_out1: Tensor<4> = add175_out1.unsqueeze_dims::<4>(&[0]);
        let unsqueeze356_out1: Tensor<4> = add178_out1.unsqueeze_dims::<4>(&[0]);
        let gather200_out1 = {
            let sliced = unsqueeze355_out1.slice(s![- 1, .., .., ..]);
            sliced.squeeze_dim::<3usize>(0)
        };
        let gather201_out1 = {
            let sliced = unsqueeze354_out1.slice(s![- 1, .., .., ..]);
            sliced.squeeze_dim::<3usize>(0)
        };
        let gather202_out1 = {
            let sliced = unsqueeze356_out1.slice(s![- 1, .., .., ..]);
            sliced.squeeze_dim::<3usize>(0)
        };
        let [split25_out1, split25_out2, split25_out3, split25_out4] = {
            let split_tensors = gather201_out1.split_with_sizes([1, 1, 1, 1].into(), 2);
            split_tensors.try_into().unwrap()
        };
        let squeeze53_out1 = split25_out1.squeeze_dims::<2>(&[-1]);
        let squeeze54_out1 = split25_out2.squeeze_dims::<2>(&[-1]);
        let squeeze55_out1 = split25_out3.squeeze_dims::<2>(&[-1]);
        let squeeze56_out1 = split25_out4.squeeze_dims::<2>(&[-1]);
        let constant1875_out1 = self.constant1875.val();
        let mul295_out1 = squeeze55_out1
            .clone()
            .mul((constant1875_out1).unsqueeze_dims(&[0isize]));
        let sub23_out1 = squeeze53_out1.clone().sub(mul295_out1);
        let constant1876_out1 = self.constant1876.val();
        let mul296_out1 = squeeze56_out1
            .clone()
            .mul((constant1876_out1).unsqueeze_dims(&[0isize]));
        let sub24_out1 = squeeze54_out1.clone().sub(mul296_out1);
        let constant1877_out1 = self.constant1877.val();
        let mul297_out1 = squeeze55_out1
            .mul((constant1877_out1).unsqueeze_dims(&[0isize]));
        let add179_out1 = squeeze53_out1.add(mul297_out1);
        let constant1878_out1 = self.constant1878.val();
        let mul298_out1 = squeeze56_out1
            .mul((constant1878_out1).unsqueeze_dims(&[0isize]));
        let add180_out1 = squeeze54_out1.add(mul298_out1);
        let unsqueeze357_out1: Tensor<3> = sub23_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze358_out1: Tensor<3> = sub24_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze359_out1: Tensor<3> = add179_out1.unsqueeze_dims::<3>(&[-1]);
        let unsqueeze360_out1: Tensor<3> = add180_out1.unsqueeze_dims::<3>(&[-1]);
        let concat185_out1 = burn::tensor::Tensor::cat(
            [unsqueeze357_out1, unsqueeze358_out1, unsqueeze359_out1, unsqueeze360_out1]
                .into(),
            2,
        );
        let expand7_out1 = {
            let onnx_shape: [i64; 2usize] = [1, 1];
            let input_dims = orig_target_sizes.dims();
            let mut shape = onnx_shape;
            #[allow(clippy::needless_range_loop)]
            for i in 0..2usize {
                let dim_offset = i;
                if shape[dim_offset] == 1 && input_dims[i] > 1 {
                    shape[dim_offset] = input_dims[i] as i64;
                }
            }
            orig_target_sizes.expand(shape)
        };
        let tile4_out1 = expand7_out1.repeat(&[1, 2]);
        let unsqueeze361_out1: Tensor<3, Int> = tile4_out1.unsqueeze_dims::<3>(&[1]);
        let cast191_out1 = unsqueeze361_out1.float().cast(burn::tensor::DType::F32);
        let mul299_out1 = concat185_out1.mul(cast191_out1);
        let sigmoid59_out1 = burn::tensor::activation::sigmoid(gather200_out1);
        let flatten3_out1 = {
            let leading_dim = sigmoid59_out1.shape()[..1].iter().product::<usize>()
                as i32;
            sigmoid59_out1.reshape::<2, _>([leading_dim, -1])
        };
        let (topk3_out1, topk3_out2) = {
            let (values, indices) = flatten3_out1.topk_with_indices(1200, 1);
            (values, indices.cast(burn::tensor::DType::I64))
        };
        let constant1886_out1 = self.constant1886.val();
        let div146_out1 = topk3_out2
            .clone()
            .div((constant1886_out1).unsqueeze_dims(&[0isize]));
        let constant1887_out1 = self.constant1887.val();
        let mul300_out1 = div146_out1
            .clone()
            .mul((constant1887_out1).unsqueeze_dims(&[0isize]));
        let sub25_out1 = topk3_out2.sub(mul300_out1);
        let unsqueeze362_out1: Tensor<3, Int> = div146_out1
            .clone()
            .unsqueeze_dims::<3>(&[-1]);
        let shape208_out1: [i64; 3] = {
            let axes = &mul299_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather203_out1 = shape208_out1[2] as i64;
        let unsqueeze364_out1 = [gather203_out1 as i64];
        let constant1893_out1: [i64; 1] = [1i64];
        let constant1894_out1: [i64; 1] = [1i64];
        let concat187_out1: [i64; 3usize] = [
            &constant1893_out1[..],
            &constant1894_out1[..],
            &unsqueeze364_out1[..],
        ]
            .concat()
            .try_into()
            .unwrap();
        let expand8_out1 = {
            let onnx_shape: [i64; 3usize] = [1, 1, 1];
            let input_dims = unsqueeze362_out1.dims();
            let mut shape = onnx_shape;
            #[allow(clippy::needless_range_loop)]
            for i in 0..3usize {
                let dim_offset = i;
                if shape[dim_offset] == 1 && input_dims[i] > 1 {
                    shape[dim_offset] = input_dims[i] as i64;
                }
            }
            unsqueeze362_out1.expand(shape)
        };
        let tile5_out1 = {
            let repeats: alloc::vec::Vec<usize> = concat187_out1
                .iter()
                .map(|&v| v as usize)
                .collect();
            expand8_out1.repeat(&repeats)
        };
        let gatherelements3_out1 = {
            let axis_size = mul299_out1.dims()[1] as i64;
            let negative = tile5_out1.clone().lower_elem(0i64);
            let corrected = tile5_out1.clone() + axis_size;
            let indices = tile5_out1.mask_where(negative, corrected);
            mul299_out1.gather(1, indices)
        };
        let sigmoid60_out1 = burn::tensor::activation::sigmoid(gather202_out1);
        let argmax_result = sigmoid60_out1.argmax(2);
        let argmax1_out1 = argmax_result
            .squeeze_dim::<2usize>(2)
            .cast(burn::tensor::DType::I64);
        let gatherelements4_out1 = {
            let axis_size = argmax1_out1.dims()[1] as i64;
            let negative = div146_out1.clone().lower_elem(0i64);
            let corrected = div146_out1.clone() + axis_size;
            let indices = div146_out1.mask_where(negative, corrected);
            argmax1_out1.gather(1, indices)
        };
        (sub25_out1, gatherelements3_out1, topk3_out1, gatherelements4_out1)
    }
}

#[derive(Module, Debug)]
pub struct Model {
    submodule1: Submodule1,
    submodule2: Submodule2,
    submodule3: Submodule3,
    submodule4: Submodule4,
    submodule5: Submodule5,
    submodule6: Submodule6,
    submodule7: Submodule7,
    submodule8: Submodule8,
    submodule9: Submodule9,
    submodule10: Submodule10,
    submodule11: Submodule11,
    submodule12: Submodule12,
    submodule13: Submodule13,
    submodule14: Submodule14,
    submodule15: Submodule15,
    submodule16: Submodule16,
    submodule17: Submodule17,
    submodule18: Submodule18,
    submodule19: Submodule19,
    submodule20: Submodule20,
    submodule21: Submodule21,
    submodule22: Submodule22,
    submodule23: Submodule23,
    submodule24: Submodule24,
    submodule25: Submodule25,
    device: AllocationDevice,
}


extern crate std;

impl Default for Model {
    fn default() -> Self {
        Self::from_file(
            "/home/antigravity/.gemini/jetski/brain/v1/default/jetski_remote_agents_3p_devbox/6706d7dd-4907-4965-8a12-02bd466ffcea/scratch/burn-poc/gen_deim/deim-s-1024x1024-261124.bpk",
            &Default::default(),
        )
    }
}

impl Model {
    /// Load model weights from a burnpack file.
    pub fn from_file<P: AsRef<std::path::Path>>(file: P, device: &Device) -> Self {
        let mut model = Self::new(device);
        let mut store = BurnpackStore::from_file(&file);
        model
            .load_from(&mut store)
            .unwrap_or_else(|e| {
                panic!("Failed to load burnpack file {}: {e}", file.as_ref().display())
            });
        model
    }

    /// Load model weights from in-memory bytes.
    ///
    /// The bytes must be the contents of a `.bpk` file.
    pub fn from_bytes(bytes: Bytes, device: &Device) -> Self {
        let mut model = Self::new(device);
        let mut store = BurnpackStore::from_bytes(Some(bytes));
        model
            .load_from(&mut store)
            .unwrap_or_else(|e| panic!("Failed to load burnpack bytes: {e}"));
        model
    }
}

impl Model {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let submodule1 = Submodule1::new(device);
        let submodule2 = Submodule2::new(device);
        let submodule3 = Submodule3::new(device);
        let submodule4 = Submodule4::new(device);
        let submodule5 = Submodule5::new(device);
        let submodule6 = Submodule6::new(device);
        let submodule7 = Submodule7::new(device);
        let submodule8 = Submodule8::new(device);
        let submodule9 = Submodule9::new(device);
        let submodule10 = Submodule10::new(device);
        let submodule11 = Submodule11::new(device);
        let submodule12 = Submodule12::new(device);
        let submodule13 = Submodule13::new(device);
        let submodule14 = Submodule14::new(device);
        let submodule15 = Submodule15::new(device);
        let submodule16 = Submodule16::new(device);
        let submodule17 = Submodule17::new(device);
        let submodule18 = Submodule18::new(device);
        let submodule19 = Submodule19::new(device);
        let submodule20 = Submodule20::new(device);
        let submodule21 = Submodule21::new(device);
        let submodule22 = Submodule22::new(device);
        let submodule23 = Submodule23::new(device);
        let submodule24 = Submodule24::new(device);
        let submodule25 = Submodule25::new(device);
        Self {
            submodule1,
            submodule2,
            submodule3,
            submodule4,
            submodule5,
            submodule6,
            submodule7,
            submodule8,
            submodule9,
            submodule10,
            submodule11,
            submodule12,
            submodule13,
            submodule14,
            submodule15,
            submodule16,
            submodule17,
            submodule18,
            submodule19,
            submodule20,
            submodule21,
            submodule22,
            submodule23,
            submodule24,
            submodule25,
            device: AllocationDevice(device.clone()),
        }
    }
    #[allow(dead_code)]
    fn device(&self) -> &Device {
        &self.device.0
    }

    #[allow(clippy::let_and_return, clippy::approx_constant)]
    /// # Arguments
    /// - `images`: expected dtype `DType::F32`
    /// - `orig_target_sizes`: expected dtype `DType::I64`
    pub fn forward(
        &self,
        images: Tensor<4>,
        orig_target_sizes: Tensor<2, Int>,
    ) -> (Tensor<2, Int>, Tensor<3>, Tensor<2>, Tensor<2, Int>) {
        let (mul3_out1, concat3_out1, gather3_out1, div1_out1, div2_out1) = self
            .submodule1
            .forward(images.clone());
        let (add3_out1, unsqueeze8_out1, unsqueeze6_out1) = self
            .submodule2
            .forward(mul3_out1, concat3_out1);
        let add8_out1 = self
            .submodule3
            .forward(add3_out1, unsqueeze8_out1.clone(), unsqueeze6_out1.clone());
        let add13_out1 = self
            .submodule4
            .forward(add8_out1, unsqueeze8_out1.clone(), unsqueeze6_out1.clone());
        let add18_out1 = self
            .submodule5
            .forward(add13_out1, unsqueeze8_out1.clone(), unsqueeze6_out1.clone());
        let (add23_out1, slice38_out1) = self
            .submodule6
            .forward(add18_out1, unsqueeze8_out1.clone(), unsqueeze6_out1.clone());
        let add28_out1 = self
            .submodule7
            .forward(add23_out1, unsqueeze8_out1.clone(), unsqueeze6_out1.clone());
        let add33_out1 = self
            .submodule8
            .forward(add28_out1, unsqueeze8_out1.clone(), unsqueeze6_out1.clone());
        let add38_out1 = self
            .submodule9
            .forward(add33_out1, unsqueeze8_out1.clone(), unsqueeze6_out1.clone());
        let (add43_out1, slice75_out1) = self
            .submodule10
            .forward(add38_out1, unsqueeze8_out1.clone(), unsqueeze6_out1.clone());
        let add48_out1 = self
            .submodule11
            .forward(add43_out1, unsqueeze8_out1.clone(), unsqueeze6_out1.clone());
        let (reshape33_out1, resize1_out1, resize2_out1) = self
            .submodule12
            .forward(
                add48_out1,
                unsqueeze8_out1,
                unsqueeze6_out1,
                slice38_out1,
                gather3_out1,
                div1_out1,
                div2_out1,
                slice75_out1,
            );
        let (reshape40_out1, conv2d7_out1, conv2d6_out1) = self
            .submodule13
            .forward(reshape33_out1, images, resize1_out1, resize2_out1);
        let (concat97_out1, conv2d18_out1, conv2d9_out1) = self
            .submodule14
            .forward(reshape40_out1, conv2d7_out1, conv2d6_out1);
        let (concat99_out1, mul134_out1, mul146_out1) = self
            .submodule15
            .forward(concat97_out1, conv2d18_out1, conv2d9_out1);
        let (
            gather107_out1,
            gather108_out1,
            gather109_out1,
            gather110_out1,
            gather111_out1,
            gather112_out1,
            reshape44_out1,
            add88_out1,
            gatherelements2_out1,
        ) = self.submodule16.forward(concat99_out1, mul134_out1, mul146_out1);
        let (
            add93_out1,
            clip1_out1,
            unsqueeze162_out1,
            slice136_out1,
            slice137_out1,
            slice138_out1,
            sigmoid35_out1,
        ) = self
            .submodule17
            .forward(
                gather107_out1.clone(),
                gather108_out1.clone(),
                gather109_out1.clone(),
                gather110_out1.clone(),
                gather111_out1.clone(),
                gather112_out1.clone(),
                reshape44_out1,
                add88_out1,
                gatherelements2_out1,
            );
        let (add98_out1, constant339_out1, shape148_out1) = self
            .submodule18
            .forward(
                add93_out1,
                clip1_out1.clone(),
                unsqueeze162_out1,
                slice136_out1.clone(),
                gather107_out1.clone(),
                gather108_out1.clone(),
                gather109_out1.clone(),
                gather110_out1.clone(),
                slice137_out1.clone(),
                gather111_out1.clone(),
                gather112_out1.clone(),
                slice138_out1.clone(),
            );
        let (shape155_out1, linear76_out1, sigmoid42_out1, mul190_out1) = self
            .submodule19
            .forward(add98_out1, sigmoid35_out1);
        let (
            add118_out1,
            constant1469_out1,
            constant100_out1,
            gather134_out1,
            gather132_out1,
            gather137_out1,
            gather135_out1,
        ) = self
            .submodule20
            .forward(
                shape155_out1,
                linear76_out1.clone(),
                sigmoid42_out1,
                mul190_out1.clone(),
                clip1_out1.clone(),
                constant339_out1.clone(),
                shape148_out1.clone(),
                gather107_out1.clone(),
                gather108_out1.clone(),
                slice136_out1.clone(),
                gather109_out1.clone(),
                gather110_out1.clone(),
                slice137_out1.clone(),
                gather111_out1.clone(),
                gather112_out1.clone(),
                slice138_out1.clone(),
            );
        let (concat141_out1, mul223_out1, add124_out1) = self
            .submodule21
            .forward(
                add118_out1,
                mul190_out1,
                linear76_out1,
                constant1469_out1.clone(),
                constant100_out1.clone(),
                gather134_out1.clone(),
                gather132_out1.clone(),
                gather137_out1.clone(),
                gather135_out1.clone(),
            );
        let (add145_out1, mul254_out1) = self
            .submodule22
            .forward(
                concat141_out1,
                mul223_out1,
                clip1_out1.clone(),
                constant339_out1.clone(),
                shape148_out1.clone(),
                gather107_out1.clone(),
                gather108_out1.clone(),
                slice136_out1.clone(),
                gather109_out1.clone(),
                gather110_out1.clone(),
                slice137_out1.clone(),
                gather111_out1.clone(),
                gather112_out1.clone(),
                slice138_out1.clone(),
                add124_out1,
            );
        let add160_out1 = self
            .submodule23
            .forward(
                add145_out1.clone(),
                constant1469_out1.clone(),
                constant100_out1.clone(),
                gather134_out1.clone(),
                gather132_out1.clone(),
                gather137_out1.clone(),
                gather135_out1.clone(),
                mul254_out1.clone(),
                clip1_out1,
                constant339_out1,
                shape148_out1,
                gather107_out1,
                gather108_out1,
                slice136_out1,
                gather109_out1,
                gather110_out1,
                slice137_out1,
                gather111_out1,
                gather112_out1,
                slice138_out1,
            );
        let (reshape110_out1, linear113_out1, mul285_out1, concat179_out1) = self
            .submodule24
            .forward(
                add160_out1,
                mul254_out1,
                add145_out1,
                constant1469_out1,
                constant100_out1,
                gather134_out1,
                gather132_out1,
                gather137_out1,
                gather135_out1,
            );
        let (sub25_out1, gatherelements3_out1, topk3_out1, gatherelements4_out1) = self
            .submodule25
            .forward(
                reshape110_out1,
                linear113_out1,
                mul285_out1,
                concat179_out1,
                orig_target_sizes,
            );
        (sub25_out1, gatherelements3_out1, topk3_out1, gatherelements4_out1)
    }
}
