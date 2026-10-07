# SPDX-FileCopyrightText: (C) 2025 Institute of Software, Chinese Academy of Sciences (ISCAS)
# SPDX-FileCopyrightText: (C) 2025 openRuyi Project Contributors
# SPDX-FileContributor: Zheng Junjie <zhengjunjie@iscas.ac.cn>
# SPDX-FileContributor: jchzhou <zhoujiacheng@iscas.ac.cn>
# SPDX-FileContributor: misaka00251 <liuxin@iscas.ac.cn>
#
# SPDX-License-Identifier: MulanPSL-2.0

Name:           efibootmgr
Version:        18
Release:        %autorelease
Summary:        EFI Boot Manager
License:        GPL-2.0-or-later
URL:            https://github.com/rhboot/efibootmgr
#!RemoteAsset:  sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
Source0:        https://github.com/rhboot/efibootmgr/archive/refs/tags/%{version}.tar.gz#/%{name}-%{version}.tar.bz2
BuildSystem:    autotools

BuildOption(build):  CFLAGS="%{optflags} -flto -fPIE -pie"
BuildOption(build):  OS_VENDOR="%{_vendor}"
BuildOption(build):  EFI_LOADER="%{LOADER}"
BuildOption(build):  EFIDIR="%{_vendor}"
BuildOption(install):  DESTDIR=%{buildroot}
BuildOption(install):  sbindir=%{_sbindir}
BuildOption(install):  EFIDIR="%{_vendor}"

BuildRequires:  pkgconfig
BuildRequires:  pkgconfig(efiboot) >= 31
BuildRequires:  pkgconfig(efivar) >= 31
BuildRequires:  pkgconfig(libpci)
BuildRequires:  pkgconfig(popt)
BuildRequires:  pkgconfig(zlib)
BuildRequires:  autoconf
BuildRequires:  automake
BuildRequires:  libtool
BuildRequires:  make

%description
The EFI Boot Manager allows the user to edit the Intel Extensible
Firmware Interface (EFI) Boot Manager variables.  Additional
information about the EFI can be found at
<http://developer.intel.com/technology/efi/efi.htm>.

%prep -a
# removing hotfix function declaration:
# https://github.com/rhboot/efibootmgr/issues/128
sed -e '/extern int efi_set_verbose/d' -i "src/efibootmgr.c"

%conf

%check

%files
%license COPYING
%doc README
%{_sbindir}/efiboot*
%{_mandir}/man8/*.gz

%changelog
%autochangelog
