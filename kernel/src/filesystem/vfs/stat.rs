use system_error::SystemError;

use crate::{
    arch::filesystem::stat::PosixStat,
    driver::base::device::device_number::DeviceNumber,
    filesystem::vfs::{mount::is_mountpoint_root, vcore::do_file_lookup_at},
    process::ProcessManager,
    syscall::user_access::UserBufferWriter,
    time::PosixTimeSpec,
};
use alloc::sync::Arc;

use super::{
    fcntl::AtFlags,
    syscall::{ModeType, PosixStatx, PosixStatxMask, StxAttributes},
    IndexNode,
};

#[derive(Clone)]
pub struct KStat {
    pub result_mask: PosixStatxMask, // What fields the user got
    pub mode: ModeType,              // umode_t
    pub nlink: u32,
    pub blksize: u32, // Preferred I/O size
    pub attributes: StxAttributes,
    pub attributes_mask: StxAttributes,
    pub ino: u64,
    pub dev: DeviceNumber,    // dev_t
    pub rdev: DeviceNumber,   // dev_t
    pub uid: u32,             // kuid_t
    pub gid: u32,             // kgid_t
    pub size: usize,          // loff_t
    pub atime: PosixTimeSpec, // struct timespec64
    pub mtime: PosixTimeSpec, // struct timespec64
    pub ctime: PosixTimeSpec, // struct timespec64
    pub btime: PosixTimeSpec, // File creation time
    pub blocks: u64,
    pub mnt_id: u64,
    pub dio_mem_align: u32,
    pub dio_offset_align: u32,
}

impl Default for KStat {
    fn default() -> Self {
        Self {
            result_mask: PosixStatxMask::empty(),
            mode: ModeType::empty(),
            nlink: Default::default(),
            blksize: Default::default(),
            attributes: StxAttributes::empty(),
            attributes_mask: StxAttributes::empty(),
            ino: Default::default(),
            dev: Default::default(),
            rdev: Default::default(),
            uid: Default::default(),
            gid: Default::default(),
            size: Default::default(),
            atime: Default::default(),
            mtime: Default::default(),
            ctime: Default::default(),
            btime: Default::default(),
            blocks: Default::default(),
            mnt_id: Default::default(),
            dio_mem_align: Default::default(),
            dio_offset_align: Default::default(),
        }
    }
}

bitflags! {
    ///  https://code.dragonos.org.cn/xref/linux-6.6.21/include/linux/namei.h?fi=LOOKUP_FOLLOW#21
    pub struct LookUpFlags: u32 {
        /// follow links at the end
        const FOLLOW = 0x0001;
        /// require a directory
        const DIRECTORY = 0x0002;
        /// force terminal automount
        const AUTOMOUNT = 0x0004;
        /// accept empty path [user_... only]
        const EMPTY = 0x4000;
        /// follow mounts in the starting point
        const DOWN = 0x8000;
        /// follow mounts in the end
        const MOUNTPOINT = 0x0080;
        /// tell ->d_revalidate() to trust no cache
        const REVAL = 0x0020;
        /// RCU pathwalk mode; semi-internal
        const RCU = 0x0040;
        /// ... in open
        const OPEN = 0x0100;
        /// ... in object creation
        const CREATE = 0x0200;
        /// ... in exclusive creation
        const EXCL = 0x0400;
        /// ... in destination of rename()
        const RENAME_TARGET = 0x0800;
        /// internal use only
        const PARENT = 0x0010;
        /// No symlink crossing
        const NO_SYMLINKS = 0x010000;
        /// No nd_jump_link() crossing
        const NO_MAGICLINKS = 0x020000;
        /// No mountpoint crossing
        const NO_XDEV = 0x040000;
        /// No escaping from starting point
        const BENEATH = 0x080000;
        /// Treat dirfd as fs root
        const IN_ROOT = 0x100000;
        /// Only do cached lookup
        const CACHED = 0x200000;
        ///  LOOKUP_* flags which do scope-related checks based on the dirfd.
        const IS_SCOPED = LookUpFlags::BENEATH.bits | LookUpFlags::IN_ROOT.bits;
    }
}

impl From<AtFlags> for LookUpFlags {
    fn from(value: AtFlags) -> Self {
        let mut lookup_flags = LookUpFlags::empty();

        if !value.contains(AtFlags::AT_SYMLINK_NOFOLLOW) {
            lookup_flags |= LookUpFlags::FOLLOW;
        }

        if !value.contains(AtFlags::AT_NO_AUTOMOUNT) {
            lookup_flags |= LookUpFlags::AUTOMOUNT;
        }

        if value.contains(AtFlags::AT_EMPTY_PATH) {
            lookup_flags |= LookUpFlags::EMPTY;
        }

        lookup_flags
    }
}

/// https://code.dragonos.org.cn/xref/linux-6.6.21/fs/stat.c#232
#[inline(never)]
pub fn vfs_statx(
    dfd: i32,
    filename: &str,
    flags: AtFlags,
    request_mask: PosixStatxMask,
) -> Result<KStat, SystemError> {
    let lookup_flags: LookUpFlags = flags.into();

    // Validate flags - only allowed flags are AT_SYMLINK_NOFOLLOW, AT_NO_AUTOMOUNT, AT_EMPTY_PATH, AT_STATX_SYNC_TYPE
    if flags.intersects(
        !(AtFlags::AT_SYMLINK_NOFOLLOW
            | AtFlags::AT_NO_AUTOMOUNT
            | AtFlags::AT_EMPTY_PATH
            | AtFlags::AT_STATX_SYNC_TYPE),
    ) {
        return Err(SystemError::EINVAL);
    }
    let inode = do_file_lookup_at(dfd, filename, lookup_flags)?;

    let mut kstat = vfs_getattr(&inode, request_mask, flags)?;
    if is_mountpoint_root(&inode) {
        kstat
            .attributes
            .insert(StxAttributes::STATX_ATTR_MOUNT_ROOT);
    }
    kstat
        .attributes_mask
        .insert(StxAttributes::STATX_ATTR_MOUNT_ROOT);

    // todo: 添加 https://code.dragonos.org.cn/xref/linux-6.6.21/fs/stat.c#266 这里的逻辑

    Ok(kstat)
}

/// 获取文件的增强基本属性
///
/// # 参数
/// - `path`: 目标文件路径
/// - `stat`: 用于返回统计信息的结构体
/// - `request_mask`: PosixStatxMask标志位，指示调用者需要哪些属性
/// - `query_flags`: 查询模式(AT_STATX_SYNC_TYPE)
///
/// # 描述
/// 向文件系统请求文件的属性。调用者必须通过request_mask和query_flags指定需要的信息。
///
/// 如果文件是远程的：
/// - 可以通过传递AT_STATX_FORCE_SYNC强制文件系统从后端存储更新属性
/// - 可以通过传递AT_STATX_DONT_SYNC禁止更新
///
/// request_mask中必须设置相应的位来指示调用者需要检索哪些属性。
/// 未请求的属性也可能被返回，但其值可能是近似的，如果是远程文件，
/// 可能没有与服务器同步。
///
/// # 返回值
/// 成功时返回0，失败时返回负的错误码
///
/// 参考 https://code.dragonos.org.cn/xref/linux-6.6.21/fs/stat.c#165
#[inline(never)]
pub fn vfs_getattr(
    inode: &Arc<dyn IndexNode>,
    request_mask: PosixStatxMask,
    mut at_flags: AtFlags,
) -> Result<KStat, SystemError> {
    if at_flags.contains(AtFlags::AT_GETATTR_NOSEC) {
        return Err(SystemError::EPERM);
    }

    let mut kstat = KStat::default();
    kstat.result_mask |= PosixStatxMask::STATX_BASIC_STATS;
    at_flags &= AtFlags::AT_STATX_SYNC_TYPE;

    let metadata = inode.metadata()?;
    if metadata.atime.is_empty() {
        kstat.result_mask.remove(PosixStatxMask::STATX_ATIME);
    }

    // todo: 添加automount和dax属性

    kstat.blksize = metadata.blk_size as u32;
    if request_mask.contains(PosixStatxMask::STATX_MODE)
        || request_mask.contains(PosixStatxMask::STATX_TYPE)
    {
        kstat.mode = metadata.mode;
    }
    if request_mask.contains(PosixStatxMask::STATX_NLINK) {
        kstat.nlink = metadata.nlinks as u32;
    }
    if request_mask.contains(PosixStatxMask::STATX_UID) {
        kstat.uid = metadata.uid as u32;
    }
    if request_mask.contains(PosixStatxMask::STATX_GID) {
        kstat.gid = metadata.gid as u32;
    }
    if request_mask.contains(PosixStatxMask::STATX_ATIME) {
        kstat.atime.tv_sec = metadata.atime.tv_sec;
        kstat.atime.tv_nsec = metadata.atime.tv_nsec;
    }
    if request_mask.contains(PosixStatxMask::STATX_MTIME) {
        kstat.mtime.tv_sec = metadata.mtime.tv_sec;
        kstat.mtime.tv_nsec = metadata.mtime.tv_nsec;
    }
    if request_mask.contains(PosixStatxMask::STATX_CTIME) {
        // ctime是文件上次修改状态的时间
        kstat.ctime.tv_sec = metadata.ctime.tv_sec;
        kstat.ctime.tv_nsec = metadata.ctime.tv_nsec;
    }
    if request_mask.contains(PosixStatxMask::STATX_INO) {
        kstat.ino = metadata.inode_id.into() as u64;
    }
    if request_mask.contains(PosixStatxMask::STATX_SIZE) {
        kstat.size = metadata.size as usize;
    }
    if request_mask.contains(PosixStatxMask::STATX_BLOCKS) {
        kstat.blocks = metadata.blocks as u64;
    }

    if request_mask.contains(PosixStatxMask::STATX_BTIME) {
        // btime是文件创建时间
        kstat.btime.tv_sec = metadata.btime.tv_sec;
        kstat.btime.tv_nsec = metadata.btime.tv_nsec;
    }
    if request_mask.contains(PosixStatxMask::STATX_ALL) {
        kstat.attributes = StxAttributes::STATX_ATTR_APPEND;
        kstat.attributes_mask |=
            StxAttributes::STATX_ATTR_AUTOMOUNT | StxAttributes::STATX_ATTR_DAX;
        kstat.dev = DeviceNumber::from(metadata.dev_id as u32);
        kstat.rdev = metadata.raw_dev;
    }

    // 把文件类型加入mode里面 （todo: 在具体的文件系统里面去实现这个操作。这里只是权宜之计）
    kstat.mode |= metadata.file_type.into();

    return Ok(kstat);
}

/// 参考 https://code.dragonos.org.cn/xref/linux-6.6.21/fs/stat.c#274
#[inline(never)]
pub fn vfs_fstatat(dfd: i32, filename: &str, flags: AtFlags) -> Result<KStat, SystemError> {
    // log::debug!("vfs_fstatat: dfd={}, filename={}", dfd, filename);
    let statx_flags = flags | AtFlags::AT_NO_AUTOMOUNT;
    if dfd >= 0 && flags == AtFlags::AT_EMPTY_PATH {
        return vfs_fstat(dfd);
    }

    return vfs_statx(
        dfd,
        filename,
        statx_flags,
        PosixStatxMask::STATX_BASIC_STATS,
    );
}

/// vfs_fstat - Get the basic attributes by file descriptor
///
/// # Arguments
/// - fd: The file descriptor referring to the file of interest
///
///  This function is a wrapper around vfs_getattr(). The main difference is
///  that it uses a file descriptor to determine the file location.
///
/// 参考: https://code.dragonos.org.cn/xref/linux-6.6.21/fs/stat.c#190
pub fn vfs_fstat(dfd: i32) -> Result<KStat, SystemError> {
    // Get the file from the file descriptor
    let pcb = ProcessManager::current_pcb();
    let fd_table = pcb.fd_table();
    let file = fd_table
        .read()
        .get_file_by_fd(dfd)
        .ok_or(SystemError::EBADF)?;
    let inode = file.inode();

    // Get attributes using vfs_getattr with basic stats mask
    vfs_getattr(&inode, PosixStatxMask::STATX_BASIC_STATS, AtFlags::empty())
}

pub(super) fn do_newfstatat(
    dfd: i32,
    filename: &str,
    user_stat_buf_ptr: usize,
    flags: u32,
) -> Result<(), SystemError> {
    let kstat = vfs_fstatat(dfd, filename, AtFlags::from_bits_truncate(flags as i32))?;

    cp_new_stat(kstat, user_stat_buf_ptr)
}

/// 参考 https://code.dragonos.org.cn/xref/linux-6.6.21/fs/stat.c#393
#[inline(never)]
pub(super) fn cp_new_stat(kstat: KStat, user_buf_ptr: usize) -> Result<(), SystemError> {
    let posix_stat = PosixStat::try_from(kstat)?;
    let mut ubuf_writer =
        UserBufferWriter::new(user_buf_ptr as *mut PosixStat, size_of::<PosixStat>(), true)?;
    ubuf_writer
        .copy_one_to_user(&posix_stat, 0)
        .map_err(|_| SystemError::EFAULT)
}

/// 参考 https://code.dragonos.org.cn/xref/linux-6.6.21/fs/stat.c#660
pub(super) fn do_statx(
    dfd: i32,
    filename: &str,
    flags: u32,
    mask: u32,
    user_kstat_ptr: usize,
) -> Result<(), SystemError> {
    let mask = PosixStatxMask::from_bits_truncate(mask);
    if mask.contains(PosixStatxMask::STATX_RESERVED) {
        return Err(SystemError::EINVAL);
    }

    let flags = AtFlags::from_bits_truncate(flags as i32);
    if flags.contains(AtFlags::AT_STATX_SYNC_TYPE) {
        return Err(SystemError::EINVAL);
    }

    let kstat = vfs_statx(dfd, filename, flags, mask)?;
    cp_statx(kstat, user_kstat_ptr)
}

/// 参考 https://code.dragonos.org.cn/xref/linux-6.6.21/fs/stat.c#622
#[inline(never)]
fn cp_statx(kstat: KStat, user_buf_ptr: usize) -> Result<(), SystemError> {
    let mut userbuf = UserBufferWriter::new(
        user_buf_ptr as *mut PosixStatx,
        size_of::<PosixStatx>(),
        true,
    )?;
    let mut statx: PosixStatx = PosixStatx::new();

    // Copy fields from KStat to PosixStatx
    statx.stx_mask = kstat.result_mask & !PosixStatxMask::STATX_CHANGE_COOKIE;
    statx.stx_blksize = kstat.blksize;
    statx.stx_attributes = kstat.attributes & !StxAttributes::STATX_ATTR_CHANGE_MONOTONIC;
    statx.stx_nlink = kstat.nlink;
    statx.stx_uid = kstat.uid;
    statx.stx_gid = kstat.gid;
    statx.stx_mode = kstat.mode;
    statx.stx_inode = kstat.ino;
    statx.stx_size = kstat.size as i64;
    statx.stx_blocks = kstat.blocks;
    statx.stx_attributes_mask = kstat.attributes_mask;

    // Copy time fields
    statx.stx_atime = kstat.atime;
    statx.stx_btime = kstat.btime;
    statx.stx_ctime = kstat.ctime;
    statx.stx_mtime = kstat.mtime;

    // Convert device numbers
    statx.stx_rdev_major = kstat.rdev.major().data();
    statx.stx_rdev_minor = kstat.rdev.minor();
    statx.stx_dev_major = kstat.dev.major().data();
    statx.stx_dev_minor = kstat.dev.minor();

    statx.stx_mnt_id = kstat.mnt_id;
    statx.stx_dio_mem_align = kstat.dio_mem_align;
    statx.stx_dio_offset_align = kstat.dio_offset_align;

    // Write to user space
    userbuf.copy_one_to_user(&statx, 0)?;
    Ok(())
}

/// 通用的PosixStat
#[allow(unused)]
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct GenericPosixStat {
    /// Device ID
    pub st_dev: u64,
    /// File serial number (inode)
    pub st_ino: u64,
    /// File mode
    pub st_mode: u32,
    /// Link count
    pub st_nlink: u32,
    /// User ID of the file's owner
    pub st_uid: u32,
    /// Group ID of the file's group
    pub st_gid: u32,
    /// Device number, if device
    pub st_rdev: u64,
    /// Padding
    pub __pad1: u64,
    /// Size of file, in bytes
    pub st_size: i64,
    /// Optimal block size for I/O
    pub st_blksize: i32,
    /// Padding
    pub __pad2: i32,
    /// Number 512-byte blocks allocated
    pub st_blocks: i64,
    /// Time of last access (seconds)
    pub st_atime: i64,
    /// Time of last access (nanoseconds)
    pub st_atime_nsec: u64,
    /// Time of last modification (seconds)
    pub st_mtime: i64,
    /// Time of last modification (nanoseconds)
    pub st_mtime_nsec: u64,
    /// Time of last status change (seconds)
    pub st_ctime: i64,
    /// Time of last status change (nanoseconds)
    pub st_ctime_nsec: u64,
    /// Unused
    pub __unused4: u32,
    /// Unused
    pub __unused5: u32,
}

/// 转换的代码参考 https://code.dragonos.org.cn/xref/linux-6.6.21/fs/stat.c#393
impl TryFrom<KStat> for GenericPosixStat {
    type Error = SystemError;

    fn try_from(kstat: KStat) -> Result<Self, Self::Error> {
        let mut tmp = GenericPosixStat::default();
        if core::mem::size_of_val(&tmp.st_dev) < 4 && !kstat.dev.old_valid_dev() {
            return Err(SystemError::EOVERFLOW);
        }
        if core::mem::size_of_val(&tmp.st_rdev) < 4 && !kstat.rdev.old_valid_dev() {
            return Err(SystemError::EOVERFLOW);
        }

        tmp.st_dev = kstat.dev.new_encode_dev() as u64;
        tmp.st_ino = kstat.ino;

        if core::mem::size_of_val(&tmp.st_ino) < core::mem::size_of_val(&kstat.ino)
            && tmp.st_ino != kstat.ino
        {
            return Err(SystemError::EOVERFLOW);
        }

        tmp.st_mode = kstat.mode.bits();
        tmp.st_nlink = kstat.nlink;

        // todo: 处理user namespace (https://code.dragonos.org.cn/xref/linux-6.6.21/fs/stat.c#415)
        tmp.st_uid = kstat.uid;
        tmp.st_gid = kstat.gid;

        tmp.st_rdev = kstat.rdev.data() as u64;
        tmp.st_size = kstat.size as i64;

        tmp.st_atime = kstat.atime.tv_sec;
        tmp.st_mtime = kstat.mtime.tv_sec;
        tmp.st_ctime = kstat.ctime.tv_sec;
        tmp.st_atime_nsec = kstat.atime.tv_nsec as u64;
        tmp.st_mtime_nsec = kstat.mtime.tv_nsec as u64;
        tmp.st_ctime_nsec = kstat.ctime.tv_nsec as u64;
        tmp.st_blocks = kstat.blocks as i64;
        tmp.st_blksize = kstat.blksize as i32;

        Ok(tmp)
    }
}
