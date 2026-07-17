# Force ggml's SIMD kernels on for MSVC x86-64 builds.
#
# Why this file exists:
#   whisper-rs-sys hardcodes its cmake defines and leaves GGML_NATIVE=ON. On
#   GCC/Clang that means -march=native and everything Just Works. MSVC has no
#   -march=native, so ggml's detection produced nothing and left
#   GGML_AVX/AVX2/AVX512 all OFF — compiling every kernel as scalar C.
#
#   Measured cost of that: 237 SECONDS to transcribe 1.5s of audio with
#   large-v3-turbo q5_0 (~160x slower than real-time). Not a hang — just scalar
#   code dequantising a q5_0 model one float at a time. Sampling strategy made
#   no difference (greedy and beam were both ~240s), which is what ruled out
#   every other theory.
#
#   Setting CXXFLAGS=/arch:AVX2 is NOT enough: ggml gates which kernels it
#   compiles on these cmake variables, not just on the compiler's __AVX2__.
#   whisper-rs-sys gives us no way to pass defines, but cmake-rs forwards
#   CMAKE_TOOLCHAIN_FILE from the environment, and a toolchain file is read
#   before ggml's options are evaluated — so this is the only seam that reaches
#   them without forking the crate.
#
# Baseline: AVX2 + FMA + F16C are Intel Haswell (2013) and all AMD Ryzen. That
# is the CPU floor this buys us; anything older would fault on an illegal
# instruction. Revisit if that ever matters.

if(NOT DEFINED SOTTO_AVX2_TOOLCHAIN_APPLIED)
  set(SOTTO_AVX2_TOOLCHAIN_APPLIED TRUE)

  # THE BIG ONE: put optimization back.
  #
  # cmake-rs strips `/O2` out of the compiler args it forwards (its `skip_arg`
  # drops anything starting with /O, meaning to "let cmake deal with
  # optimization"), and then writes what's left into CMAKE_<LANG>_FLAGS_RELEASE
  # — which *overrides* CMake's own `/O2 /Ob2 /DNDEBUG` default for that build
  # type. MSVC with no /O flag means /Od. cmake-rs's own source comments that
  # this "overrides things like the optimization flags, which is bad".
  #
  # Net effect before this: whisper.cpp compiled with optimization OFF. 217s to
  # transcribe one 30s chunk. Nothing about it looks broken — it just runs ~20x
  # slow, and every profile says "compute-bound" because it truthfully is.
  #
  # FORCE is load-bearing: cmake-rs passes its version as -D on the command
  # line, which pre-seeds the cache, so only a FORCE'd set in a toolchain file
  # (processed later) can win.
  foreach(lang C CXX)
    set(CMAKE_${lang}_FLAGS_RELEASE "/MD /O2 /Ob2 /DNDEBUG" CACHE STRING "" FORCE)
    set(CMAKE_${lang}_FLAGS_RELWITHDEBINFO "/MD /Zi /O2 /Ob1 /DNDEBUG" CACHE STRING "" FORCE)
  endforeach()

  set(GGML_NATIVE OFF CACHE BOOL "no -march=native on MSVC; set flags explicitly" FORCE)
  set(GGML_AVX    ON  CACHE BOOL "" FORCE)
  set(GGML_AVX2   ON  CACHE BOOL "" FORCE)
  set(GGML_FMA    ON  CACHE BOOL "" FORCE)
  set(GGML_F16C   ON  CACHE BOOL "" FORCE)
  # AVX-512 stays off on purpose: narrow CPU support, and on many parts it
  # downclocks enough to be a net loss.
  set(GGML_AVX512 OFF CACHE BOOL "" FORCE)
endif()
