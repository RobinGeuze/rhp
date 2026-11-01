<?php
// openssl public keys: PEM/cert/PKCS#1 parsing, details, RSA (PKCS#1 v1.5) +
// ECDSA verification across digests, error returns. Fixtures made with php.
$K = array (
  'rsa' => 
  array (
    'pub' => '-----BEGIN PUBLIC KEY-----
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQC8dJ1adN1iO6N5NkABfiX4aUQe
/4Fwl8ieHHju1OZzTCXeyahCs52FOvFTYOQEl4RbA2iVQx78nXt7iTk+mgZ1+tpr
xBx1QV0LDEcLLd2OaWmhFLAV5dlqnv0sbWyQX6hO4Q+be15nf/bpAo4f2Zuc8x9i
BRhJ8KfkQ/zJ0fiwVQIDAQAB
-----END PUBLIC KEY-----
',
    'sigs' => 
    array (
      'sha1' => 'SL7YLDzSJuju66Ma4MQg4QO4NHcV3Ry9yZoct4BKcALXnOfjjGN8vuMCkuPQSq/2u9onsGWTvW5TxI6iw3pI0QfS7qhc8jzJfdS89CKnVQKNiH1gRxgecxBO3cjHYhttJk9w44zl9UCDo9BiIA/AoSXopT2XKN8uxaDuCcpmjbc=',
      'sha256' => 'T4nNTV/m3gKkoAB6TqCQ1G8lI+uDQU/9rIz8nKm26mRirvksHpJ5V4eF0Qt90+vgQRSP0jI52vGQBFAzS4hMZS1EVYIz7+WRV/tHhI8ckGp9Wz/26HgbFMs4em6zGwmway7xSouxOVhVCaa2VTK2E29AUeRk+VVG6ImIfqMKZ7o=',
      'sha384' => 'n0mbMMs9xd9w/VpquvLGPhlHrUQ/FNFxjOAijzCpBOF1+wgcTWSJmUJIZEUfPWVVYTQbb0dlp8qovYjD87C+mtKrXNlGMppgq216YYjC9Ogi0AzNgn25ZlawlJ1yd0VGR0xOnetN8bIeVZtVW5aaELxRwalZfF902Cyuvi1Jl18=',
      'sha512' => 'Sm03tXae9X9mT/O6JVVG30OYPNRs0ex6ITXNM4BAt11nhVtCS9u1XlfNg+AkTgw21mEcHvLzZPrkrIvcTI5CMjcCJNzB009vr7RGn9t4BrUHY2+fmZciBpgiLt3DmPIrAcL5r/9VFyD/xi7CmLTornWYcl9YjQbeMtZ2uKkyUco=',
      'md5' => 't2pNLd+2pCIZgujMleNuqgPOiXY9e1h4+LSeQ7pG8I2bqXtfb+Mu8dnYg+mpCCFICzk79HUreK4lufWnsSFA5vZ6YmiJlNXfbTN43EwIcY0MxsoN/S/VXbnG5IZpBF+bwu9N4TGM/spILzW8z7+e9/XtMoqH+GmSAwXN3BCBBSw=',
      'sha224' => 'Pz5qb0pKqNVR9lZTMdBIO0hlbx4lnoQIoITz34i/s+oJ0R4D9nUpTuiDGD8R402fyWGIwbSk6iRy0iMcxlaNCqDhQMQ65L2BhiIPlgigKAfh2wfbRNAHu63ZFDFAK/omjQUujOWZsOa6vv7mOGQtDShUSib5HZzU+uOLhTULPbk=',
      'rmd160' => 'Upvy5g2BofJY2IXtJUhN1Y+iH6SJfMIgtZo4VxHYjgAjreYGpC541/ZiFld3g+te6MK0nVzt1XxXKBuQt6L0QxLfZJIFAFwZfHAidKvckYNY2IUEfM96qKSgoTvPgtjy8zCsD3hs6/NfHv19tXk0KguFxqAg5t3vLfUfsANuhuQ=',
      'sha3-256' => 'hZ5F/AXIywqoKIuTqsV1j04bMaOxq3Ox9V1Gu8Yfkg0darHsE2PseCL4DEwHYi3ogXBjjXHJoFys65qKzvmFvnF3p4SI86DCdqSy5pNGSkzfBSw0vX5n9pAY1pomhwoJSfIaZQLB0XzfSo9zKdcdZPXxP4PavatE6A42XO7EQx8=',
    ),
    'cert' => '-----BEGIN CERTIFICATE-----
MIICgTCCAeqgAwIBAgIBADANBgkqhkiG9w0BAQUFADBcMRUwEwYDVQQDDAxleGFt
cGxlLnRlc3QxCzAJBgNVBAYTAkFVMRMwEQYDVQQIDApTb21lLVN0YXRlMSEwHwYD
VQQKDBhJbnRlcm5ldCBXaWRnaXRzIFB0eSBMdGQwHhcNMjYxMDMxMjE0NTUzWhcN
MjcxMDMxMjE0NTUzWjBcMRUwEwYDVQQDDAxleGFtcGxlLnRlc3QxCzAJBgNVBAYT
AkFVMRMwEQYDVQQIDApTb21lLVN0YXRlMSEwHwYDVQQKDBhJbnRlcm5ldCBXaWRn
aXRzIFB0eSBMdGQwgZ8wDQYJKoZIhvcNAQEBBQADgY0AMIGJAoGBALx0nVp03WI7
o3k2QAF+JfhpRB7/gXCXyJ4ceO7U5nNMJd7JqEKznYU68VNg5ASXhFsDaJVDHvyd
e3uJOT6aBnX62mvEHHVBXQsMRwst3Y5paaEUsBXl2Wqe/SxtbJBfqE7hD5t7Xmd/
9ukCjh/Zm5zzH2IFGEnwp+RD/MnR+LBVAgMBAAGjUzBRMB0GA1UdDgQWBBRl5/WK
bUbiYN992MZda0Bzfv2rWzAfBgNVHSMEGDAWgBRl5/WKbUbiYN992MZda0Bzfv2r
WzAPBgNVHRMBAf8EBTADAQH/MA0GCSqGSIb3DQEBBQUAA4GBAKJZyOsZ3Akgyzsv
boEZtsUP+aMUWhPdn7WfdsrSm0cFEEypvDces28TYo+VM59zbcO4IXqZ/XMZVUdr
CNAIjJZgbpC5Khl1OOBO9c9catI6bmub8pv0I2mduoUdgFgraApeLz7CdL6hOeLY
vf01i3ki7mSKDubAR1jaKrF/LwWl
-----END CERTIFICATE-----
',
    'priv' => '-----BEGIN PRIVATE KEY-----
MIICeAIBADANBgkqhkiG9w0BAQEFAASCAmIwggJeAgEAAoGBALx0nVp03WI7o3k2
QAF+JfhpRB7/gXCXyJ4ceO7U5nNMJd7JqEKznYU68VNg5ASXhFsDaJVDHvyde3uJ
OT6aBnX62mvEHHVBXQsMRwst3Y5paaEUsBXl2Wqe/SxtbJBfqE7hD5t7Xmd/9ukC
jh/Zm5zzH2IFGEnwp+RD/MnR+LBVAgMBAAECgYEAir7oYoKLwxjZYBI6yV6QofUO
PCwspjJ5dz7LIjsId3Zmo0GgaYtHu6ELrzE0btUGkuNNiibil2VyVh9PT/GiMtAS
WOgzsSAC7Y2CD2Te2xsKQwhImPfxGEFLcgi5ln3rUUO8Nj/U9OXuPGiZRACuPa+0
k03BBpthlDBqfJb2NYECQQD6OciGRduzcCD/f40yVFx6HWPCjNabqNXNHLqmPrmo
OYztSR5WZobsglOS0MW7a+SoZKU6biFL3ZDeG/7uJpLxAkEAwM3sAvlCneUzu7Bi
5bexv+Zpg7Hjvg4oKusnqBGFZLUKj3qrpLD3z2zNxJnKP7TeqbteaYFFaLfdPXSo
NEWrpQJBANDyy5YEY8/VRFjWZEuw0oBOZm2hOdxGAjMDWIFWQ6ZMWHgU5LZ1axm4
Qp2IpHU7RXewg6DlMfyuCjjO2/R2VaECQDcMlI5kzPh4MMUuH46nXEEGcKoBaJ0j
PaTDb5c1KXkPl+xPgiKLxLWcw0OXBYAbDFg5/0LwffjgRnL9k+kY22ECQQD32XY3
6F9o0KL/AlPQf0m2Nzp99JydjBr1wL34GyrBapldl5sgg3knCtWvSOvlctjpwdOh
7TjcGTZnfjLQQZWh
-----END PRIVATE KEY-----
',
  ),
  'rsa2048' => 
  array (
    'pub' => '-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAqSpxOovBa8G/Ea6UnWe3
s+W4S44Vch4kTURAuSzMr7YXU29Msu9KajNtUe6R55U+pdEX9DXWWodGI+7p2tNE
FGGoBvRSvgPG0plkKK7Nkv5pNfl7y0ZHG3YQoBGs5lSd3ezx6s/S053BbwdM4pLg
KuQ4V1R/lEHu2FReCOuuq9o/9qFO7p9W9JzQaXPD6PwalkVHNOCNFbiv82CHY4FY
6Q1u44KRvla9EqmRyb+X9+2vU8XNtmzPkDQJNdyR3VfPFe32CpMf4qymV2PvOIaI
xbpfEPEn9rL1DPeCN64ULyIr1YTVisEYJyhY5UNHDbEKtJHIXwIlG14wkiFwWlW+
wwIDAQAB
-----END PUBLIC KEY-----
',
    'sigs' => 
    array (
      'sha1' => 'fWZtbRL+Y6ks8upYMpwyDy9+29kVMJvBDpmpAL4bNVS2RrIFn3RLF/z5d2jHzCV99Jo0JanyHIztpiVItdg3P+cgE1bM1rNZOT7LGQb2fsk/0FNa5S8X25Himh5WL/6W4PiZtOx1AEsMr2vuA4XcBqF5e+ChiashxNC6h88OUkXQvz+PQfrI775aGV1pvjuoBvc6d/lg7RQgiGoLyPy7/50q0492HeVDpyTrdRZXZfD0l6i8fo3bqYqBqQ6EQWzFBQiEwVST/GACBG7K6Ir9ESb38sK+0yVLizYTI121JFLK3JS34PF19H91/CpFWObIleqW/Pu/O7I1CxxZEUZOsw==',
      'sha256' => 'nJxhPhYlUOis+EBgRY9wPNIdR3lE2Au3IEo1LIfdNrU0KOTHoq/jECC3ljmoxaBdZIgMUHpDCKh44ZAyy00TJ/7yTxHIcNvOA5Ky1Oiosyngm49Jrw2oUeDSRDD5w6KsQ26DguTt5WaTJvdV3Si2veXRw8CPyh2f07VLerB+FARQFx97GyCt9wgONe4bh/2Jv1hVJlPItXIr6YFoq8o0yfoF7pGqxt/Aco6O9J3CeWfPIRLnVIYYhjsasYj0CG2F1VdgeM/iMve610d8oa5mU+oV76HwJDoRsgC9cdEQn8zbigRjESeHkKYxkMHKgaIaCnhqPO9p+6sEzCLLb3c6RA==',
      'sha384' => 'DP5Mo811a+pbrws4b9LhdqYbUVm+ZFAKqqPqlhtITzz3oYs/y6/oS7W281L7T3r8qHr2IEAhuBKt1HbToEOZmETc7wiR73BlMLlVjr7MWx2VBlSPnZJbtwwaU/3O8xbEWQAPd1uyZzxM1Ri/IMTcI7EFF5QOPv0Ib8H+BqGJ5yeqFkPM1c+E3JzosKuYpnjKScOu9bLBiVkBMuXWw2Sbcmc5miIy6a0ljyQI6pWRO+WDxRPUIp6JsG8llY7zed069kSWMTEmAdVLcxDNNT8iYvr/qmfqLKJAtsReysejDqTCT8HX88JV48dKgp74mBHa4c5gUCnAnvpzQHHZaZ0QnQ==',
      'sha512' => 'JoPn/mPmhxTV1cyBMhqJgvzq3Sd/cbp5cxEF6YYPjQ+9sqj91tZltJUmJBTelf/YvxIMw4BF3UdH8NSz3oh5GwsBU6BAN5nUOhNZDSbuG3SDMHYxFY6dQm19KpLHufm3xbc7YJ9084v9j3PXQtyqkusDLuQTTvQ3ZVIYm+EM02//a95FiYTmsFSEEXwCN903Kp1Y+r1ygk8bsdx674pI1Z8yeSF99JqOLPG1EWyxXTtsvAmSZTGAZuXrr4GSRYBOiumnNvX03bX+VQ+FFPRQS6wsGroVhG1APtloVRK/E05yqS/Om1iWfhDYeTgxdEy3pdXeV0OqZowOlGrl4c8Auw==',
      'md5' => 'WeIksy01vsRV1DTXAI9KMrczvzSsd5ywfuQJACKbSA8psJoRFFAchQSj4RxRg/CgpmpJXXE18OXbzw7hZ1RpDriOYsNTVCI9GRKVSxDVc/voHuunSngVQY+KevsAvCoOwhPOm3YIdaJGeh58rD3y9rFkDYZxDpGKwmqkHWlf2kKt+dv7iuk3W9guN6ufhqqaW/JzcMXOrUvog2CdmbFqYVev0CfqwQzMiP4XIIDk8x2bcigEWqVaU+lcrlInddKQiIG11HJ6YueeVSWqNbphRXPcfdqTz6zWeLsnoFBKSl4H4U5vA/JXs71CmuiNet0CPDyn784I4CHUIFMMNt9VCg==',
      'sha224' => 'Ir/ZVqOdyGRQ+RHtN7UChGZnsHZve6KUJ0L+6UoGBdf5ym+F4JciPikJ0qO54LuhpeZ80gdaqf6ETwwl8MCTf5GBKuyafTJ2MbTDxTuFFC34JUvR403Id9zXP7/TfyESw0q8x245yE0msSLTGWy/Z/iKB41ZiMCxAQXeZ3g/12rITYUYDNHxvTqx9hpQ7On2wNu4+TET7qGbaMHKoz55RJJNw5o71PXtSSqezK8VM314Q/M0QNVJYzIefnb7MH2S/HQ3BFl0ubHG3S4+60FJL4kwdaiYs0BCgLqUSSPVes/aRkCY/EXiiiKJYhNEzWH0224UTjrjiuyoSjiJnpf+9g==',
      'rmd160' => 'FyEErk8yGVVoALkiGhfZ7KKrjfjAwioEeiDCxpo89EQnlR7SrUAy4meijQH2aCpT1RSwMWx5+ANlH33MWR/1hZZsP8EuMjBy60YwMPKuJCoIoTB60jQUAAahIHrC1DDndkDIqF8FdPcXaiqGSsKV0j5ZVSKmbWuRimVIb/cUSUrlO6dGSoY8sPWB20NVBRcHpmFjF/Ol0O2Nx0xgIYg5K8jkSaEOa31g7YrppSaNHAoWzWQexFXrtAMYWV7d6mxe4OFCIlWcoqQuf/Itp9k/wH3tKMvQzu3+Qutu5e8lCCW3AqAsWcoflhMj6EZM5woZ6tGFYJ7k9XAOR/1E8c7oNA==',
      'sha3-256' => 'KPfazPUOvvElHMXKxIjgIi5yj78J/p2lj8CxTEY2wlwUOwzEC9pNs17amv4dU6aS9srhPzM3yqmb4lGmjoGXrR9HyDEQ9jP87ytDUY+JL3+Zf40tMu8NvDZ9V/Gi06S5UDZBFP8/ObohOGOriSTAT3WCHOyO670quVyiWNjuktf3CJ5HjTEwZ5lmuTOU/wcr3lok4QjSdHErwIat2tCu6RBrTmsEbGe+vxoLMhQWB+yNTqsehYQVRmWER7xDht2tKrVLje7/dpUKgFKYoVKIct3+tCz/3Yfi5NEFXlRQQVsKA+8zWp/THAzfVb8tfY3OTKdfYaNDyaCmJVDNzTtz0g==',
    ),
  ),
  'ec256' => 
  array (
    'pub' => '-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEb0juCnk6R2lIWkq0bSWzPBsoImrB
O4bihc90FOdq4LXCZ3weD9YelPP4XpA8W+gswhpe+pBkgCqdPqTf8vN72g==
-----END PUBLIC KEY-----
',
    'sigs' => 
    array (
      'sha1' => 'MEQCIFxMntRtgLC4L64CdohEYOSVlhK12So1VpvztXxBd/r7AiAPUUPIQ14MT0Juckm28URKCjYXk8SWrEJk5+/0jWz/Pg==',
      'sha256' => 'MEYCIQCeDBjGv00skRrACs9fccJ5XkXhh/dGyigCWCO7BzUEaQIhALlW5o/+01Bp2yU3Msp7fOtrrash+K3/Vk9nqIMKZkww',
      'sha384' => 'MEUCIHVjnSB67+j03ldizKOGTRHu5LYm3XHhyrXQVddIB04CAiEAoJ26MHf/DqfYqPEHkYvFvDbJBPPx3OaxIvXVLUH/H80=',
      'sha512' => 'MEYCIQDAjGMveZL+pJelOrGmzLIXXJQ0aLZ+OJzJpMoGTB65hwIhALR8mW9ZXi53bH7Z80go/hUBiY09fL1g3MQWQX391si2',
      'md5' => 'MEYCIQDSQvgPFKvj8dmYKjN981d6vnvrMxqUBdJLji85eAbM8wIhALaklHCGMIYkjtoQcwfRQO/nKbvvT+1cclHE+K2t7/jO',
      'sha224' => 'MEQCICJ5mpfrvPEZKLGW0hZ9xkALoXMnJQsJckApkwZCKWUEAiAx8cpQ5krPTSXdRc4LGrvv8EAVavOupJeEBc/D+etjXw==',
      'rmd160' => 'MEUCIQDj2YVb2zcR0Yvek4ak8RoV6fRRetmE4ITadNjtXLlC3gIge7dqsPVTR8gOz046uFmXCh9w2lhWC/OljC548//lIrY=',
      'sha3-256' => 'MEUCIHIhpBKLrOYFiRpNMF4UiUH+di1wV9an+v/LE5S5mAasAiEA6kbGuhaAeox1Hbl0vWM86Y7QAVzB8OStXx6QXOyWAWk=',
    ),
  ),
  'ec384' => 
  array (
    'pub' => '-----BEGIN PUBLIC KEY-----
MHYwEAYHKoZIzj0CAQYFK4EEACIDYgAEwNdFIbKelQfNZ/7qnPbBP7p9X5EUc4M7
VK+SsM/rHh3pLxj4DxAagXNqt9X8Bm2Py4EVysjDRIF6Q7/eqDsNRdfr1TSuECF9
Ake0q4VlIAc0LcwnWJslRoCWhbzaiRtN
-----END PUBLIC KEY-----
',
    'sigs' => 
    array (
      'sha1' => 'MGUCMCD51w/+u8xhzBlm7kW5VHto7lZo1oJy9wCNkvivh5XBeIeaDzwwSrt2WKzfjqI53wIxAMRBgwsXX+SxmGaLHVbG3IZcMuUkss9W2aHUF0bpwoEyCFsgiStnqxd+rCggAamLkg==',
      'sha256' => 'MGYCMQC0LqxVYEG8Eed9LbQ8dlVq99AH0PQngG9qwGHBzKm6DKK5cfNNqQSHecdupWMW+2QCMQCarP+U7nLhJ+IZ5oFaY4CiaThcsS6gGtyD1gv9e32CQP1oKsgMI1PFkkfTJuRW7wc=',
      'sha384' => 'MGQCMH+BPD9uu8JVvsUnsW4KtdzieqYJjcVz7Tc2Ar0Qb1t0vM2mId2UFnhHZ1yJ6YPmfgIwSsDtIvhhXFosRDiaq+uYI48Dydz/61DTHEcMCj2hbWeCkRzrC4JtlgZxeA/Kq3t2',
      'sha512' => 'MGUCMHffYZsyG26PrdgjIpO688avmyyEuegiMo3khkYfoEM47SqbOME/tqM66T/eUmnypgIxAN6u2Zjvx3PyuKcx6UZmrsjN+ysZwSZomY4oVzIkZouNEaPv5Iv3TTl2/bd0snEIpg==',
      'md5' => 'MGYCMQDcKiWmlbBaba0v+WzoiKaSlYJrX2+Pu8pXEaRmCraB76c3e+7Bkzx7VGYPeQhoqR4CMQDjxxBYxG8L6AYnF3iMz6fiF/0E4vv1Hc97Y/fHlI3tRu1zfOLnzs5hl8MIm73+2zw=',
      'sha224' => 'MGYCMQChg1RoGkLSBnGu+bmGg309FkRoEA8T5i5onNa39G16SOOI53hoappy/2M6N02SekwCMQC8BkYiGD+oAQe/TJwb6OeDP02Fpb8iDAS+HoueUvPWRS2zJh6s1igKT8w1vkqLkM0=',
      'rmd160' => 'MGYCMQCoZj9iiSzbjRcALGrlXu2UibaM4pGXFFZp4OBAoxIXOMLb45Amn37H1dKfrOrCyqQCMQDrLMAHTCB+IpMNpnJ/xNL1zZTMVEq+u0+G7zpEr0aS3iEevKFtY+t7uXuAT4OzqfA=',
      'sha3-256' => 'MGUCMBQeUezP+yoLiZBZJqWh0jdV4qN3AZeVrTBxRMQ6t7KxJaoHZoXXQHuCMj+sribesQIxAOI3o4l5lisNtO49AIwhD0RjPcCG7LCQmudyP6/8O+jFSvWx0m4baP436MMkkjnI/g==',
    ),
  ),
);

function show($d) {
    if (!is_array($d)) { var_dump($d); return; }
    array_walk_recursive($d, function (&$v) { if (is_string($v) && !preg_match('/^[\x20-\x7e\n]*$/', $v)) $v = 'hex:' . bin2hex($v); });
    var_export($d); echo "\n";
}
foreach ($K as $name => $info) {
    echo "== $name\n";
    $k = openssl_pkey_get_public($info['pub']);
    var_dump(get_class($k));
    show(openssl_pkey_get_details($k));
    foreach ($info['sigs'] as $alg => $sig) {
        echo "$alg: ", var_export(openssl_verify("hello world", base64_decode($sig), $k, $alg), true), " ";
        echo var_export(openssl_verify("hello worlD", base64_decode($sig), $info['pub'], $alg), true), "\n";
    }
}
$r = $K['rsa'];
$k2 = openssl_get_publickey($r['cert']);
show(openssl_pkey_get_details($k2));
var_dump(openssl_verify("hello world", base64_decode($r['sigs']['sha1']), $r['cert']));
var_dump(openssl_verify("hello world", base64_decode($r['sigs']['sha256']), $k2, OPENSSL_ALGO_SHA256));
var_dump(openssl_verify("hello world", base64_decode($r['sigs']['sha256']), $k2, "SHA256"));
var_dump(openssl_verify("hello world", base64_decode($r['sigs']['sha256']), $k2, "sha256WithRSAEncryption"));
var_dump(openssl_verify("hello world", base64_decode($r['sigs']['sha256']), $k2, "RSA-SHA256"));
var_dump(openssl_verify("hello world", base64_decode($r['sigs']['sha256']), $k2, 0));
var_dump(openssl_verify("hello world", base64_decode($r['sigs']['sha1']), $k2, OPENSSL_ALGO_SHA1, OPENSSL_PKCS1_PADDING));
var_dump(openssl_verify("hello world", base64_decode($r['sigs']['sha256']), [$k2, "pass"], OPENSSL_ALGO_SHA256));
var_dump(openssl_verify("hello world", "short", $k2, OPENSSL_ALGO_SHA256));
var_dump(openssl_verify("hello world", "", $k2));
var_dump(openssl_verify("hello world", base64_decode($r['sigs']['sha256']), $K['rsa2048']['pub'], OPENSSL_ALGO_SHA256));
var_dump(openssl_verify("hello world", base64_decode($K['ec256']['sigs']['sha256']), $K['ec384']['pub'], OPENSSL_ALGO_SHA256));
var_dump(openssl_verify("hello world", "garbage", $K['ec256']['pub'], OPENSSL_ALGO_SHA256));
var_dump(openssl_verify("hello world", base64_decode($r['sigs']['sha256']), $k2, OPENSSL_ALGO_SHA256, 0));
$pss256 = base64_decode("cqaPXclST9I/XW8PS/RvsXIwds7eHOsStNQXc6X2Atw+Su9jkOMevw8r/huN4FKrDQZ5Riqt80JvNz+2vacL+wKCQwy7bVonnAuliNBmfHujzfDSuBfDczahoSNMqHF5/fwG8bmmeiuio5b7mwEElkmkQEYmXhRCmiGEepHXmZk=");
$pss1 = base64_decode("Yw2gnPYUbYY2Xqwg60P7noY4hlAATmSt3mEof4/zC127/TFZ4AXshGbFmvlCOFtlkESqkl876HSM/BQJE2byiLeQkrMI3315N8E3lnta9LDcZnbm63ajdYKFjwazl+dmuwDnKucTfr+0foWi3xrqsbHcYFIrPPN8TbqdA6hCJMw=");
var_dump(openssl_verify("hello world", $pss256, $k2, "sha256", OPENSSL_PKCS1_PSS_PADDING));
var_dump(openssl_verify("hello world", $pss1, $k2, OPENSSL_ALGO_SHA1, OPENSSL_PKCS1_PSS_PADDING));
var_dump(openssl_verify("hello worle", $pss256, $k2, "sha256", OPENSSL_PKCS1_PSS_PADDING));
var_dump(openssl_verify("hello world", $pss256, $k2, "sha256"));
var_dump(openssl_verify("hello world", $pss1, $k2, "sha256", OPENSSL_PKCS1_PSS_PADDING));
var_dump(openssl_pkey_get_public("garbage"));
var_dump(openssl_pkey_get_public($r['priv']));
var_dump(openssl_pkey_get_public(""));
var_dump(openssl_pkey_get_public(123));
var_dump(get_class(openssl_pkey_get_public($k2)));
var_dump(get_class(openssl_pkey_get_public([$r['pub'], ""])));
try { openssl_pkey_get_public([]); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { new OpenSSLAsymmetricKey(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { openssl_pkey_get_details("x"); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { openssl_pkey_get_public([1]); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { openssl_verify("a", "b", $k2, []); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$md = openssl_get_md_methods();
var_dump(count($md), in_array('sha384', $md), count(openssl_get_md_methods(true)));
echo implode(",", $md), "\n";
echo implode(",", openssl_get_md_methods(true)), "\n";
$rsapub = "-----BEGIN RSA PUBLIC KEY-----\n" . chunk_split(base64_encode(substr(base64_decode(str_replace(["-----BEGIN PUBLIC KEY-----", "-----END PUBLIC KEY-----", "\n"], "", $r['pub'])), 22)), 64, "\n") . "-----END RSA PUBLIC KEY-----\n";
$k3 = openssl_pkey_get_public($rsapub);
var_dump($k3 !== false);
echo openssl_pkey_get_details($k3)['key'];
var_dump(openssl_verify("hello world", base64_decode($r['sigs']['sha1']), $rsapub));
$f = sys_get_temp_dir() . '/rhp_golden_openssl_pkey_' . getmypid() . '.pem';
file_put_contents($f, "junk before\n" . $K['ec256']['pub']);
var_dump(openssl_pkey_get_details(openssl_pkey_get_public("file://" . $f))['bits']);
unlink($f);
var_dump(openssl_pkey_get_public("file://" . $f));
var_dump(OPENSSL_ALGO_SHA1, OPENSSL_ALGO_SHA384, OPENSSL_KEYTYPE_RSA, OPENSSL_KEYTYPE_EC, OPENSSL_PKCS1_PSS_PADDING);
